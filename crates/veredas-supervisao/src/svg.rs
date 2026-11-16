//! The layout asset: an SVG document read as XML, never by a pattern over its text. What the
//! binding needs of it is which elements carry which ids, and which of them a marker can follow;
//! what the page needs is the text as written.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The SVG namespace a layout's root is in.
pub const SVG_NS: &str = "http://www.w3.org/2000/svg";

/// The elements a marker can follow: those whose outline the page can measure along.
pub const FOLLOWABLE: [&str; 7] = ["path", "line", "polyline", "polygon", "rect", "circle", "ellipse"];

// Decided (docs/questions.md Q25, the operator, 2026-11-15): layouts are drawn in Inkscape and read
// as it saves them. The page draws on the subset below; the reader refuses the rest by name rather
// than draw it otherwise than the reference says, and drops Inkscape's own elements (`EDITOR`).

/// Inkscape's namespaces, as it saves a drawing.
pub const INKSCAPE_NS: &str = "http://www.inkscape.org/namespaces/inkscape";
pub const SODIPODI_NS: &str = "http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd";
pub const RDF_NS: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
pub const CC_NS: &str = "http://creativecommons.org/ns#";
pub const DC_NS: &str = "http://purl.org/dc/elements/1.1/";

/// Inkscape's own elements, by namespace, with where each stands as Inkscape writes it, by
/// position: `svg` the drawing's root itself, `defs` and `metadata` the root's own (children of it):
/// its editor's state (`sodipodi:namedview` and what it holds, a path effect or a perspective in
/// `defs`) beside the drawing or in the root's `defs`; the metadata block (`rdf:RDF`, holding
/// `cc:Work` and `dc:*`) in the root's `metadata`; `cc:*` and `dc:*` nowhere but inside that block.
/// Each is dropped on read, unread, what it holds with it, by the reader and by the
/// page; one standing anywhere else is refused, and so is every other foreign namespace (and, of
/// the RDF namespace, any element but `RDF_ROOT`). Inkscape's attributes on the drawing's elements
/// (`inkscape:label`, `sodipodi:nodetypes`, ...) are read by nothing but the script-link check every
/// attribute meets: no other check of the reader's and no rule of the page's names one.
pub const EDITOR: [(&str, &[&str]); 5] = [(SODIPODI_NS, &["svg", "defs"]), (INKSCAPE_NS, &["svg", "defs"]), (RDF_NS, &["metadata"]), (CC_NS, &[]), (DC_NS, &[])];

/// The one element of the RDF namespace Inkscape writes in the root's `metadata`: `rdf:RDF`.
pub const RDF_ROOT: &str = "RDF";

/// Inkscape's script beside a mesh gradient and beside an SVG2 hatch, which a browser cannot draw,
/// by the id Inkscape gives each (Q27, decided 2026-11-15: "Drop the script, load it"). Where
/// Inkscape writes it (a `script` of the root's own, writing nothing but its id and a type) it is
/// dropped on read with all it holds, as Inkscape's other elements are, and the mesh or the hatch is
/// drawn as the browser draws it, which is nothing; any other script is refused.
pub const POLYFILLS: [&str; 2] = ["mesh_polyfill", "hatch_polyfill"];

/// The elements the page draws an object on: a group, the shapes it paints, a text.
pub const DRAWN: [&str; 9] = ["g", "rect", "circle", "ellipse", "path", "polygon", "polyline", "line", "text"];

/// What may stand between an object's element (or a marker's path) and the drawing's root: groups
/// alone. Anything else draws it otherwise or not at all (a nested viewport, what is drawn only where
/// something refers to it, a text's content, a shape, a `switch`), or takes its click (a link).
pub const AROUND: [&str; 1] = ["g"];

/// SMIL's animation elements: an animated drawing would rewrite what the page writes (an element's
/// class, a marker's transform, the geometry a level or a marker is measured on), so a layout is a
/// still drawing and holds none.
pub const ANIMATIONS: [&str; 6] = ["animate", "animateColor", "animateMotion", "animateTransform", "discard", "set"];

/// Where an element carrying an id stands, for the binding's checks on the elements it draws on.
#[derive(Clone, Debug, Default)]
pub struct Placed {
    /// The ids the elements around it carry, nearest first.
    pub around: Vec<String>,
    /// The nearest element around it that is not one of `AROUND` (the drawing's own root aside).
    pub apart: Option<String>,
    /// The first element inside it that is not one of `HOLDS` (a `use`, a link, a nested viewport).
    pub holds: Option<String>,
    /// Whether a `style` attribute on it or inside it marks a declaration `!important`.
    pub important: bool,
    /// The nearest element, itself or around it, that hides it (`display: none`, as an attribute or in
    /// a `style`: Inkscape's hidden layer), as named in a refusal.
    pub hidden: Option<String>,
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
    // The page lays its own frame over the drawing's root (its tags, labels and values, its levels
    // rising up the screen): a root that moves, turns or scales itself would turn the drawing out
    // from under that frame (Q25).
    if root.attribute("transform").is_some() {
        return Err(format!("{} writes a `transform` on its root <svg>: the page lays its own frame over the drawing's root, which may not move", path.display()));
    }
    // How the root fits and clips its frame is the page's: a `preserveAspectRatio` that slices the
    // drawing would cut what the page writes at its edges, an `overflow` that shows what lies past
    // it would draw it over the page's own controls (Q25: Inkscape writes neither on a root).
    let framed = ROOT_FRAMED.iter().map(|a| a.to_string()).find(|a| root.attribute(a.as_str()).is_some()).or_else(|| {
        root.attribute("style").and_then(|s| declared(s).into_iter().find(|p| ROOT_FRAMED.iter().any(|a| { let a = a.to_ascii_lowercase(); *p == a || p.starts_with(&format!("{a}-")) })))
    });
    if let Some(attr) = framed {
        return Err(format!("{} writes `{attr}` on its root <svg>: the page fits the drawing into its frame and keeps it cut there", path.display()));
    }
    // The document type's attribute defaults (`<!ATTLIST`), which a browser applies and this
    // reader does not: an element's namespace, a link or a handler added unseen. (A parameter
    // entity's reference, the other way to declare unseen, is no XML this reader reads.)
    let prolog = &text[..root.range().start];
    if prolog.contains("<!ATTLIST") {
        return Err(format!("{} declares attribute defaults (`<!ATTLIST`) in its document type: a browser would add attributes this check never sees", path.display()));
    }
    // A reference in the document type (`&`: a character reference, an entity's, a predefined one).
    // XML replaces the character references of an entity's value where the entity is declared, so
    // `&#60;` there is markup wherever the entity is referenced, where this reader's parser reads a
    // character. With none, an entity's value is its text as written, which both read alike (its
    // markup an element this reader sees, refused below). Inkscape writes no entity.
    if prolog.find("<!DOCTYPE").is_some_and(|at| prolog[at..].contains('&')) {
        return Err(format!("{} writes a reference (`&`) in its document type: a browser expands an entity's value holding one otherwise than this reader (a character reference there becomes markup); Inkscape writes none", path.display()));
    }
    let mut ids: BTreeMap<String, (String, usize)> = BTreeMap::new();
    let mut placed: BTreeMap<String, Placed> = BTreeMap::new();
    // Inkscape's own elements, each dropped with all it holds where Inkscape writes it: what a
    // dropped element holds is known by the tree (its ancestors), never by where its text stands,
    // which an entity's nodes share with every other reference to it.
    let mut dropped: Vec<roxmltree::NodeId> = Vec::new();
    for node in root.descendants().filter(|n| n.is_element()) {
        let name = node.tag_name().name();
        if node.ancestors().any(|a| dropped.contains(&a.id())) {
            continue;
        }
        // An element an entity of the document type expands to (its text standing in the document
        // type, before the root): Inkscape writes none, and two parsers need not expand one alike,
        // so the drawing's elements are written in it (an entity of text alone is read).
        if node.range().start < root.range().start {
            return Err(format!("{} draws a <{name}> from an entity of its document type (`<!ENTITY`): Inkscape writes none; write the drawing's elements in the drawing itself", path.display()));
        }
        if let Some((ns, places)) = EDITOR.iter().find(|(ns, _)| node.tag_name().namespace() == Some(*ns)) {
            // Judged by position: the root itself, or the root's own `defs` or `metadata`.
            let parent = node.parent_element();
            let at = |p: roxmltree::Node, place: &str| p.tag_name().namespace() == Some(SVG_NS) && if place == "svg" { p == root } else { p.tag_name().name() == place && p.parent_element() == Some(root) };
            if !parent.is_some_and(|p| places.iter().any(|place| at(p, place))) {
                let within = parent.map(|p| p.tag_name().name().to_string()).unwrap_or_default();
                return Err(format!("{} holds an Inkscape element (<{name}> of `{ns}`) inside a <{within}>, where Inkscape writes none: its editor's state stands beside the drawing (a child of its root) or in the root's `defs`, its metadata in the root's `metadata`", path.display()));
            }
            // Of the RDF namespace, Inkscape writes its `RDF` alone there (the metadata's `cc:*` and
            // `dc:*` inside it, dropped with it).
            if *ns == RDF_NS && name != RDF_ROOT {
                return Err(format!("{} holds an Inkscape element (<{name}> of `{ns}`) in the root's <metadata>, where Inkscape writes its <{RDF_ROOT}> alone", path.display()));
            }
            dropped.push(node.id());
            continue;
        }
        // Inkscape's script for a mesh or a hatch, where and as Inkscape writes it (`POLYFILLS`):
        // dropped, never run (the page removes every script before it draws, and neither of the
        // host's policies runs one).
        let polyfill = name == "script"
            && node.tag_name().namespace() == Some(SVG_NS)
            && node.parent_element() == Some(root)
            && node.attribute("id").is_some_and(|id| POLYFILLS.contains(&id))
            && node.attributes().all(|a| a.namespace().is_none() && matches!(a.name(), "id" | "type"));
        if polyfill {
            dropped.push(node.id());
            continue;
        }
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
            return Err(format!("{} holds a <{name}> element of the namespace `{ns}`: a layout is a drawing in the SVG namespace (Inkscape's own elements apart), with no foreign content", path.display()));
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
        if ANIMATIONS.contains(&name) {
            return Err(format!("{} holds a <{name}> element: a layout is a still drawing, the page drawing its states and moving its markers itself", path.display()));
        }
        // With no sheet of its own, a drawing's class meets only the page's own rules (Q25).
        if node.attribute("class").is_some() {
            return Err(format!("{} holds a `class` (on <{name}>): with no sheet of its own a drawing's class meets only the page's own rules", path.display()));
        }
        if let Some(id) = node.attribute("id").filter(|id| the_pages_id(id)) {
            return Err(format!("{} holds the id `{id}` (on <{name}>), which the page names its own", path.display()));
        }
        // The page's own attributes beside its ids: it tags what it adds for an object
        // (`data-for`) and reads a marker's key (`data-key`) by its `data-` attributes, which
        // Inkscape writes none of (Q25). Any, in any case: the page may name more.
        if let Some(attr) = node.attributes().find(|a| the_pages_attribute(a.name())) {
            return Err(format!("{} holds `{}` (on <{name}>), a `data-` attribute, which the page names its own: it tags what it adds and reads a marker's key by them", path.display(), attr.name()));
        }
        // What moves, times or unpins what the page writes, however the drawing writes it (Q25).
        let moving = node.attributes().map(|a| a.name().to_string()).find(|a| MOVING_ATTRIBUTES.contains(&a.as_str())).or_else(|| node.attribute("style").and_then(|st| declared(st).into_iter().find(|p| moves_the_page(p))));
        if let Some(property) = moving {
            return Err(format!("{} holds `{property}` (on <{name}>), which would move, time or unpin what the page writes: the page places, animates and takes the clicks itself", path.display()));
        }
        // What the page's reset (`all: initial`) leaves to inheritance, CSS keeping both out of `all`:
        // a custom property, which the page's state colours read (each a `var(--…)`), and a
        // `direction`, which would turn what the page writes beside an element. Inkscape writes no
        // custom property, and `direction` on a text and its spans alone (`DIRECTED`).
        let styled = node.attribute("style").map(declared).unwrap_or_default();
        if let Some(property) = styled.iter().find(|p| p.starts_with("--")) {
            return Err(format!("{} holds the custom property `{property}` (on <{name}>), which the page's own colours would read: write the drawing's paint by its properties", path.display()));
        }
        if !DIRECTED.contains(&name) && (node.attribute("direction").is_some() || styled.iter().any(|p| p == "direction")) {
            return Err(format!("{} holds `direction` (on <{name}>), which would reach what the page writes beside the drawing: Inkscape writes it on a text and its spans alone", path.display()));
        }
        // A picture is embedded in the drawing (Q23): an image's `href`, plain or `xlink:`, is a
        // `data:image/…` URI, read as a browser reads a URL. Any other the page would load from where
        // it points. (What else could make the page load from elsewhere the host's policy bars.)
        if name == "image" {
            let linked = node.attributes().filter(|a| a.name() == "href" && (a.namespace().is_none() || a.namespace() == Some(XLINK_NS))).map(|a| a.value()).find(|v| !embedded(v));
            if let Some(href) = linked {
                return Err(format!("{} holds an <image> linking its picture (`{}`), which the page would load from where it points: embed it in Inkscape (on import, choose \"Embed\"; for a picture already linked, Extensions › Images › Embed Images)", path.display(), href.trim()));
            }
        }
        if let Some(id) = node.attribute("id") {
            let e = ids.entry(id.to_string()).or_insert_with(|| (name.to_string(), 0));
            e.1 += 1;
            placed.entry(id.to_string()).or_insert_with(|| {
                let up: Vec<roxmltree::Node> = node.ancestors().skip(1).filter(|a| a.is_element() && *a != root).collect();
                let important = |n: &roxmltree::Node| n.attribute("style").is_some_and(says_important);
                Placed {
                    around: up.iter().filter_map(|a| a.attribute("id").map(String::from)).collect(),
                    apart: up.iter().map(|a| a.tag_name().name()).find(|n| !AROUND.contains(n)).map(String::from),
                    holds: node.descendants().skip(1).filter(|d| d.is_element()).map(|d| d.tag_name().name()).find(|n| !HOLDS.contains(n)).map(String::from),
                    important: node.descendants().filter(|d| d.is_element()).any(|d| important(&d)),
                    hidden: node.ancestors().filter(|a| a.is_element()).find(|a| a.attribute("display").is_some_and(|v| style_values(&format!("display:{v}"), "display").iter().any(|x| x == "none")) || a.attribute("style").is_some_and(|s| style_values(s, "display").iter().any(|v| v == "none"))).map(|a| match a.attribute("id") {
                        Some(id) => format!("<{}> `{id}`", a.tag_name().name()),
                        None => format!("<{}>", a.tag_name().name()),
                    }),
                }
            });
        }
    }
    // The root holds what Inkscape writes on it and nothing else (Q25): its size, its view, its
    // version and its id (`ROOT_ATTRIBUTES`), and the editor's own attributes and the XML ones. Any
    // other is the page's to set: a `style` placing or sizing its box past its frame (over the page's
    // own controls), a presentation attribute hiding the drawing, how it fits.
    if let Some(a) = root.attributes().find(|a| match a.namespace() {
        None => !ROOT_ATTRIBUTES.contains(&a.name()),
        Some(ns) => ns != XML_NS && !EDITOR.iter().any(|(e, _)| *e == ns),
    }) {
        return Err(format!("{} writes `{}` on its root <svg>, which Inkscape writes none of there: the page sets the root's box and how it shows itself (Inkscape's root holds its size, its view, its version and its id, and Inkscape's own attributes)", path.display(), a.name()));
    }
    Ok(Layout { file: path.to_path_buf(), text: text.to_string(), ids, placed })
}

/// The attributes of no namespace Inkscape writes on a drawing's root, the only ones it may hold.
pub const ROOT_ATTRIBUTES: [&str; 5] = ["width", "height", "viewBox", "version", "id"];

/// The XML namespace (`xml:space`, `xml:lang`), whose attributes a root may hold.
pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// A `style` attribute read as CSS reads it (CSS Syntax, the parts a declaration's name and its
/// priority turn on): its line breaks first made one (section 3.3: a CR LF pair, a CR and a form feed each a
/// line feed); its comments as white space; its escapes decoded (one to six hex digits and the white
/// space that may end them, or any other character as itself); a string, quoted either way, and an
/// unquoted `url(…)` read as text to their ends, a `/*`, a `;`, a `:` or a `!` inside them no syntax.
/// Each character with whether it is text rather than syntax (an escaped `!`, `:`, `;` or space is a
/// letter; so is every character of a string or a url).
fn css_read(style: &str) -> Vec<(char, bool)> {
    let style = style.replace("\r\n", "\n").replace(['\r', '\x0c'], "\n");
    let cs: Vec<char> = style.chars().collect();
    let mut out: Vec<(char, bool)> = Vec::new();
    // An escape at `i` (its backslash): the character it writes pushed, the index past it returned.
    let escape = |i: usize, out: &mut Vec<(char, bool)>| -> usize {
        let hex: String = cs[i + 1..].iter().take(6).take_while(|c| c.is_ascii_hexdigit()).collect();
        if !hex.is_empty() {
            if let Some(c) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                out.push((c, true));
            }
            let past = i + 1 + hex.len();
            if cs.get(past).is_some_and(|c| css_space(*c)) { past + 1 } else { past }
        } else {
            if let Some(c) = cs.get(i + 1).filter(|c| **c != '\n') {
                out.push((*c, true));
            }
            i + 2
        }
    };
    let ident = |c: &(char, bool)| c.1 || c.0.is_alphanumeric() || c.0 == '-' || c.0 == '_' || !c.0.is_ascii();
    let mut i = 0;
    while i < cs.len() {
        if cs[i] == '/' && cs.get(i + 1) == Some(&'*') {
            let mut j = i + 2;
            while j < cs.len() && !(cs[j] == '*' && cs.get(j + 1) == Some(&'/')) {
                j += 1;
            }
            out.push((' ', false));
            i = j + 2;
            continue;
        }
        if cs[i] == '\\' {
            i = escape(i, &mut out);
            continue;
        }
        if cs[i] == '"' || cs[i] == '\'' {
            let q = cs[i];
            out.push((q, true));
            i += 1;
            while i < cs.len() && cs[i] != q && cs[i] != '\n' {
                if cs[i] == '\\' {
                    i = if cs.get(i + 1) == Some(&'\n') { i + 2 } else { escape(i, &mut out) };
                    continue;
                }
                out.push((cs[i], true));
                i += 1;
            }
            if cs.get(i) == Some(&q) {
                out.push((q, true));
                i += 1;
            }
            continue;
        }
        // `url(` (the name in any case, escaped or not, standing alone) then no quote: a url token.
        let n = out.len();
        if cs[i] == '(' && n >= 3 && out[n - 3..].iter().map(|c| c.0.to_ascii_lowercase()).eq("url".chars()) && (n == 3 || !ident(&out[n - 4])) {
            let mut j = i + 1;
            while j < cs.len() && css_space(cs[j]) {
                j += 1;
            }
            if cs.get(j) != Some(&'"') && cs.get(j) != Some(&'\'') {
                out.push(('(', false));
                i = j;
                while i < cs.len() && cs[i] != ')' {
                    if cs[i] == '\\' {
                        i = escape(i, &mut out);
                        continue;
                    }
                    out.push((cs[i], true));
                    i += 1;
                }
                continue;
            }
        }
        out.push((cs[i], false));
        i += 1;
    }
    out
}

/// CSS's white space: space, tab, line feed, carriage return, form feed.
fn css_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

/// Whether a `style` attribute's declarations say `!important` as CSS reads them: a `!` written as
/// itself, then any white space or comments, then the word `important` in any case, its letters
/// written or escaped, the word ending there. (Inside a string or a url it is no priority, as CSS
/// reads it: text, not syntax.)
pub fn says_important(style: &str) -> bool {
    let out = css_read(style);
    let letter = |(c, escaped): (char, bool)| escaped || c.is_alphanumeric() || c == '-' || c == '_' || !c.is_ascii();
    (0..out.len()).any(|k| {
        if out[k] != ('!', false) {
            return false;
        }
        let mut j = k + 1;
        while j < out.len() && !out[j].1 && css_space(out[j].0) {
            j += 1;
        }
        let word: String = out[j..].iter().take(9).map(|(c, _)| c.to_ascii_lowercase()).collect();
        word == "important" && out.get(j + 9).is_none_or(|c| !letter(*c))
    })
}

/// The properties a `style` attribute declares, as CSS reads them: each declaration's name (before
/// its `:`, between its `;`s, white space and comments around it dropped), lowercase, a vendor's
/// prefix (`VENDOR_PREFIXES`) taken off. A name with white space or a comment
/// inside it is no declaration (CSS drops it): `trans/**/form` declares nothing.
pub fn declared(style: &str) -> Vec<String> {
    let out = css_read(style);
    let mut names = Vec::new();
    for decl in out.split(|c| *c == (';', false)) {
        let Some(colon) = decl.iter().position(|c| *c == (':', false)) else { continue };
        let written = |c: &(char, bool)| c.1 || !css_space(c.0);
        let raw = &decl[..colon];
        let (Some(start), Some(end)) = (raw.iter().position(written), raw.iter().rposition(written)) else { continue };
        if raw[start..=end].iter().any(|c| !written(c)) {
            continue;
        }
        let name: String = raw[start..=end].iter().map(|(c, _)| c.to_ascii_lowercase()).collect();
        let bare = VENDOR_PREFIXES.iter().find_map(|p| name.strip_prefix(p)).unwrap_or(&name).to_string();
        if !bare.is_empty() {
            names.push(bare);
        }
    }
    names
}

/// Every value a `style` attribute declares for a property, as CSS reads them (each lowercase and
/// trimmed, a priority apart), in order: a reader that fails closed asks whether any says a value,
/// since which one a browser keeps turns on priorities and values it cannot parse.
pub fn style_values(style: &str, property: &str) -> Vec<String> {
    let out = css_read(style);
    let written = |c: &(char, bool)| c.1 || !css_space(c.0);
    let mut found = Vec::new();
    for decl in out.split(|c| *c == (';', false)) {
        let Some(colon) = decl.iter().position(|c| *c == (':', false)) else { continue };
        let raw = &decl[..colon];
        let (Some(start), Some(end)) = (raw.iter().position(written), raw.iter().rposition(written)) else { continue };
        if raw[start..=end].iter().any(|c| !written(c)) {
            continue;
        }
        let name: String = raw[start..=end].iter().map(|(c, _)| c.to_ascii_lowercase()).collect();
        if name == property {
            let value: String = decl[colon + 1..].iter().map(|(c, _)| *c).collect();
            found.push(value.split('!').next().unwrap_or("").trim().to_ascii_lowercase());
        }
    }
    found
}

/// What an engineer does in Inkscape to make an element the page draws an object on, for one it
/// does not: a clone unlinked, an image drawn or moved out.
pub fn inkscape_hint(name: &str) -> &'static str {
    match name {
        "use" => " (a clone: unlink it in Inkscape, Edit › Clone › Unlink Clone)",
        "image" => " (an image: draw it as shapes, or move it out of the object's group)",
        _ => "",
    }
}

/// Whether a CSS property moves, times or unpins what the page writes on the drawing: a transform
/// of any kind (the page places a marker by its `transform`), a transition, an animation or a
/// motion offset (the page sets what an element shows and when), `pointer-events` (the page takes
/// a click on an object's element), and `all`, which resets every one of them (a marker's
/// placement undone as `transform: none` would). Each listed property by its name and every
/// longhand of it (its name, a `-` and more: `transform-origin-x`, which Blink reads).
pub fn moves_the_page(property: &str) -> bool {
    MOVING.iter().chain(MOVING_FAMILIES.iter()).any(|p| property == *p || property.starts_with(&format!("{p}-")))
}

/// The properties `moves_the_page` names by their family: a transition's, an animation's and a motion
/// offset's, each by its name and every longhand of it.
pub const MOVING_FAMILIES: [&str; 3] = ["transition", "animation", "offset"];

/// The properties `moves_the_page` names whole, each with any longhand of it too.
pub const MOVING: [&str; 10] = ["all", "transform", "translate", "rotate", "scale", "transform-origin", "transform-box", "transform-style", "perspective", "pointer-events"];

/// What the root may not write of how it fits and clips its frame (as an attribute; `overflow` in its
/// `style` too, by its name and every longhand of it): the page fits the drawing and keeps it cut.
pub const ROOT_FRAMED: [&str; 2] = ["preserveAspectRatio", "overflow"];

/// The vendors' prefixes `declared` takes off a property's name: each engine reads its own spelling
/// of a property as the property (`-moz-transform` in Firefox, `-webkit-transform` in Blink).
pub const VENDOR_PREFIXES: [&str; 4] = ["-webkit-", "-moz-", "-ms-", "-o-"];

/// The presentation attributes of those properties SVG lets an element write (its own `transform`
/// attribute aside: the page reads and keeps it).
pub const MOVING_ATTRIBUTES: [&str; 2] = ["transform-origin", "pointer-events"];

/// Where a drawing may write `direction` (as an attribute or in a `style`): a text and what it holds,
/// as Inkscape writes it. Anywhere else it would reach the page's reset holders, which CSS's `all`
/// leaves it to.
pub const DIRECTED: [&str; 3] = ["text", "tspan", "textPath"];

/// The ids the page names its own beside the drawing's: its root (`app`) and every one it writes
/// into the drawing (the hatch pattern, a level's clip), all `sv-`; as written (the page is in standards
/// mode, where an id's case is its own: `APP` is not the page's).
pub fn the_pages_id(id: &str) -> bool {
    id == "app" || id.starts_with("sv-")
}

/// The attributes the page names its own on the drawing: every `data-` attribute, in any case (it
/// tags what it adds for an object by `data-for` and reads a marker's key by `data-key`).
pub fn the_pages_attribute(name: &str) -> bool {
    name.len() >= 5 && name.as_bytes()[..5].eq_ignore_ascii_case(b"data-")
}

/// What may stand inside an object's element: the elements the page draws on, a text's spans, and a
/// title or a description (which draw nothing).
pub const HOLDS: [&str; 12] = ["g", "rect", "circle", "ellipse", "path", "polygon", "polyline", "line", "text", "tspan", "title", "desc"];

/// The XLink namespace, in which Inkscape writes an image's `href`.
pub const XLINK_NS: &str = "http://www.w3.org/1999/xlink";

/// Whether an image's `href` embeds its picture: a `data:image/…` URI, read as a browser reads a URL
/// (tabs and line breaks anywhere dropped, leading controls and spaces trimmed), in any case.
pub fn embedded(href: &str) -> bool {
    let kept: String = href.chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect();
    kept.trim_start_matches(|c: char| c <= ' ').to_ascii_lowercase().starts_with("data:image/")
}

/// Whether an attribute's value is, or holds as an item of a `;`-separated list (an animation's
/// `values`), a `javascript:` URL as a browser parses one (the URL standard drops tabs and line
/// breaks anywhere and trims leading C0 controls and spaces), in any case.
pub fn script_link(value: &str) -> bool {
    let kept: String = value.chars().filter(|c| !matches!(c, '\t' | '\n' | '\r')).collect();
    kept.split(';').any(|item| item.trim_start_matches(|c: char| c <= ' ').to_ascii_lowercase().starts_with("javascript:"))
}

