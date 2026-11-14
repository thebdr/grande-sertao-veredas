//! The layout asset: an SVG document read as XML, never by a pattern over its text. What the
//! binding needs of it is which elements carry which ids, and which of them a marker can follow;
//! what the page needs is the text as written.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The SVG namespace a layout's root is in.
pub const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// The elements a marker can follow: those whose outline the page can measure along.
pub const FOLLOWABLE: [&str; 7] = ["path", "line", "polyline", "polygon", "rect", "circle", "ellipse"];

// DECISION PENDING (docs/questions.md Q25): the drawing the page draws on is the subset below; the
// reader refuses the rest rather than drawing it otherwise than the reference says.

/// The elements the page draws an object on: a group, the shapes it paints, a text.
pub const DRAWN: [&str; 9] = ["g", "rect", "circle", "ellipse", "path", "polygon", "polyline", "line", "text"];

/// The elements inside which the page cannot measure or paint an element as it is drawn: a viewport
/// of its own (an `svg` inside the drawing), what is drawn only where something refers to it
/// (`defs`, `symbol`, `marker`, `pattern`, `mask`, `clipPath`), and a text's content.
pub const APART: [&str; 8] = ["svg", "defs", "symbol", "marker", "pattern", "mask", "clipPath", "text"];

/// Where an element carrying an id stands, for the binding's checks on the elements it draws on.
#[derive(Clone, Debug, Default)]
pub struct Placed {
    /// The ids the elements around it carry, nearest first.
    pub around: Vec<String>,
    /// The nearest element of `APART` around it (the drawing's own root aside).
    pub apart: Option<String>,
    /// Whether a `use` is inside it (its shapes stand in another tree, which no rule reaches).
    pub holds_use: bool,
    /// Whether a `style` attribute on it or inside it marks a declaration `!important`.
    pub important: bool,
}

/// A layout as read: the file, its text, and every element id with the element's name and how
/// many elements carry it.
#[derive(Clone, Debug)]
pub struct Layout {
    pub file: PathBuf,
    pub text: String,
    /// id → (the element's local name, how many elements carry it).
    pub ids: BTreeMap<String, (String, usize)>,
    /// id → where its (first) element stands.
    pub placed: BTreeMap<String, Placed>,
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
    // The page reads the drawing as SVG: an `svg` root outside the SVG namespace is no drawing
    // a browser lays out.
    if root.tag_name().namespace() != Some(SVG_NS) {
        return Err(format!("{} is not an SVG document: its root is not in the SVG namespace ({SVG_NS})", path.display()));
    }
    // The document type's attribute defaults (`<!ATTLIST`), which a browser applies and this
    // reader does not: an element's namespace, a link or a handler added unseen. (A parameter
    // entity's reference, the other way to declare unseen, is no XML this reader reads.)
    let prolog = &text[..root.range().start];
    if prolog.contains("<!ATTLIST") {
        return Err(format!("{} declares attribute defaults (`<!ATTLIST`) in its document type: a browser would add attributes this check never sees", path.display()));
    }
    let mut ids: BTreeMap<String, (String, usize)> = BTreeMap::new();
    let mut placed: BTreeMap<String, Placed> = BTreeMap::new();
    for node in root.descendants().filter(|n| n.is_element()) {
        let name = node.tag_name().name();
        if name == "script" || name == "foreignObject" {
            return Err(format!("{} holds a <{name}> element: a layout is a drawing, with no script and no foreign content", path.display()));
        }
        // A sheet of the drawing's own would style the page's every screen, and outrank the state's
        // paint where it says so (Q25).
        if name == "style" {
            return Err(format!("{} holds a <style> sheet, which the page would apply to its own screens too: write the drawing's paint on its elements", path.display()));
        }
        // Foreign content in any other form: an element of another namespace (an XHTML frame,
        // a `path` of no namespace a marker would follow) is no drawing either.
        if node.tag_name().namespace() != Some(SVG_NS) {
            let ns = node.tag_name().namespace().unwrap_or("none");
            return Err(format!("{} holds a <{name}> element of the namespace `{ns}`: a layout is a drawing in the SVG namespace, with no foreign content", path.display()));
        }
        if let Some(attr) = node.attributes().find(|a| a.name().len() > 2 && a.name().as_bytes()[..2].eq_ignore_ascii_case(b"on")) {
            return Err(format!("{} holds an event handler (`{}` on <{name}>): a layout is a drawing, with no script", path.display(), attr.name()));
        }
        // A script link in any attribute (an `href`, an animation's `to` or `values`), read as a
        // browser reads a URL: tabs and line breaks anywhere dropped, leading controls and spaces
        // trimmed.
        if let Some(attr) = node.attributes().find(|a| script_link(a.value())) {
            return Err(format!("{} holds a script link (`{}` on <{name}>): a layout is a drawing, with no script", path.display(), attr.value().trim()));
        }
        if let Some(id) = node.attribute("id") {
            let e = ids.entry(id.to_string()).or_insert_with(|| (name.to_string(), 0));
            e.1 += 1;
            placed.entry(id.to_string()).or_insert_with(|| {
                let up: Vec<roxmltree::Node> = node.ancestors().skip(1).filter(|a| a.is_element() && *a != root).collect();
                let important = |n: &roxmltree::Node| n.attribute("style").is_some_and(|s| s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase().contains("!important"));
                Placed {
                    around: up.iter().filter_map(|a| a.attribute("id").map(String::from)).collect(),
                    apart: up.iter().map(|a| a.tag_name().name()).find(|n| APART.contains(n)).map(String::from),
                    holds_use: node.descendants().skip(1).any(|d| d.is_element() && d.tag_name().name() == "use"),
                    important: node.descendants().filter(|d| d.is_element()).any(|d| important(&d)),
                }
            });
        }
    }
    Ok(Layout { file: path.to_path_buf(), text: text.to_string(), ids, placed })
}

/// Whether an attribute's value is, or holds as an item of a `;`-separated list (an animation's
/// `values`), a `javascript:` URL as a browser parses one (the URL standard drops tabs and line
/// breaks anywhere and trims leading C0 controls and spaces), in any case.
pub fn script_link(value: &str) -> bool {
    let kept: String = value.chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect();
    kept.split(';').any(|item| item.trim_start_matches(|c: char| c <= ' ').to_ascii_lowercase().starts_with("javascript:"))
}

