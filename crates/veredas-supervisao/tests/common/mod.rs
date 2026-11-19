//! The plant supervisão's tests meet: the reference's worked example (`docs/binding-schema.md`)
//! written into a folder, its project pointed at the bench's OPC UA server, its log written by the
//! test; and raw HTTP and SSE over real sockets.
#![allow(dead_code)]

pub mod browser;

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;
use veredas_maquinar::bench::{self, Bench};

pub fn wait(within: Duration, mut what: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < within {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    what()
}

pub fn doc(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs").join(name);
    std::fs::read_to_string(p).unwrap().replace("\r\n", "\n")
}

/// The fenced blocks of a reference in a language, in order.
pub fn blocks(name: &str, lang: &str) -> Vec<String> {
    let text = doc(name);
    let fence = format!("```{lang}\n");
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find(&fence) {
        let body = &rest[start + fence.len()..];
        let end = body.find("```").unwrap();
        out.push(body[..end].to_string());
        rest = &body[end + 3..];
    }
    out
}

/// The worked example's four blocks: the supervisão file, the project, the binding, the layout.
#[derive(Clone)]
pub struct Example {
    pub host: String,
    pub project: String,
    pub binding: String,
    pub svg: String,
}

pub fn example() -> Example {
    let t = blocks("binding-schema.md", "toml");
    let s = blocks("binding-schema.md", "svg");
    assert_eq!((t.len(), s.len()), (3, 1), "the reference holds three TOML blocks and one SVG block");
    Example { host: t[0].clone(), project: t[1].clone(), binding: t[2].clone(), svg: s[0].clone() }
}

/// The example written into `dir` under the names its files give.
pub fn write_example(dir: &Path, ex: &Example) -> PathBuf {
    std::fs::create_dir_all(dir.join("loop")).unwrap();
    std::fs::write(dir.join("supervisao.toml"), &ex.host).unwrap();
    std::fs::write(dir.join("loop/project.toml"), &ex.project).unwrap();
    std::fs::write(dir.join("loop/binding.toml"), &ex.binding).unwrap();
    std::fs::write(dir.join("loop/layout.svg"), &ex.svg).unwrap();
    dir.join("supervisao.toml")
}

/// The bench of the example: an OPC UA server `plc` holding the example's tags, on a port the
/// system picks, every value set by the test (no draw), `release3` writable by a client.
pub fn plant(dir: &Path) -> Bench {
    plant_with(dir, "")
}

/// The same plant with `more` appended to its bench file (another controller, say).
pub fn plant_with(dir: &Path, more: &str) -> Bench {
    let text = &format!("{}{more}", "format = \"veredas-bench\"\nversion = 1\nseed = 5\n\n[[opc]]\nname = \"plc\"\nport = 0\ntags = [\n  { id = \"speed\", type = \"float\", value = \"1.5\" },\n  { id = \"p1\", type = \"float\", value = \"25\" },\n  { id = \"p2\", type = \"float\", value = \"75\" },\n  { id = \"fill1\", type = \"float\", value = \"35\" },\n  { id = \"fill2\", type = \"float\", value = \"85\" },\n  { id = \"fill3\", type = \"float\", value = \"100\" },\n  { id = \"infeed\", type = \"int\", value = \"1\" },\n  { id = \"release3\", type = \"int\", value = \"0\", access = \"read-write\" },\n]\n");
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("bench.toml"), text).unwrap();
    Bench::start(bench::load(text, dir).unwrap()).unwrap()
}

/// The example's text pointed at a running bench: the endpoint, the namespace the server
/// assigned, the host on a port the system picks, the writes armed or not, the poll at 50 ms.
pub fn live_example(bench: &Bench, armed: bool) -> Example {
    let plc = bench.server("plc").unwrap();
    let mut ex = example();
    ex.host = ex.host.replace("bind = \"127.0.0.1:8902\"", "bind = \"127.0.0.1:0\"");
    ex.project = ex
        .project
        .replace("opc.tcp://127.0.0.1:4855/", &plc.endpoint_url)
        .replace("ns=2;", &format!("ns={};", plc.namespace))
        .replace("poll_ms = 200", "poll_ms = 50")
        .replace("writes_armed = false", &format!("writes_armed = {armed}"));
    ex.binding = ex.binding.replace("ns=2;", &format!("ns={};", plc.namespace));
    ex
}

/// Two plants in one folder: `loop` as the reference has it, and `hall`, the same plant with
/// another name, other values, no layout and no markers.
pub fn two_plants(dir: &Path, a: &Bench, b: &Bench) -> std::path::PathBuf {
    let ex = live_example(a, false);
    write_example(dir, &ex);
    let hall = live_example(b, false);
    std::fs::create_dir_all(dir.join("hall")).unwrap();
    std::fs::write(dir.join("hall/project.toml"), hall.project.replace("name = \"the loop\"", "name = \"the hall\"")).unwrap();
    let mut binding = hall.binding.replace("title = \"The loop\"", "title = \"The hall\"").replace("layout = \"layout.svg\"\n", "");
    let cut = binding.find("[[object]]\nid = \"parcel-1\"").unwrap();
    let rest = binding[cut..].find("[[dashboard]]").unwrap() + cut;
    binding.replace_range(cut..rest, "");
    std::fs::write(dir.join("hall/binding.toml"), binding).unwrap();
    let host = format!("{}\n[[session]]\nname = \"hall\"\nproject = \"hall/project.toml\"\nbinding = \"hall/binding.toml\"\n", ex.host);
    std::fs::write(dir.join("supervisao.toml"), host).unwrap();
    dir.join("supervisao.toml")
}

/// The store's axis now: the plant's local clock read as UTC.
pub fn store_now() -> i64 {
    veredas_portal::serve::store_now_ms()
}

/// A line stamped at `ms` on the store's axis, as the example's log reads it.
pub fn stamp(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms).unwrap().naive_utc().format("%Y-%m-%d %H:%M:%S%.3f").to_string()
}

pub fn append(path: &Path, lines: &[String]) {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    for l in lines {
        f.write_all(l.as_bytes()).unwrap();
        f.write_all(b"\n").unwrap();
    }
    f.flush().unwrap();
}

fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some((size, after)) = rest.split_once("\r\n") {
        let n = usize::from_str_radix(size.trim(), 16).unwrap_or(0);
        if n == 0 {
            break;
        }
        out.push_str(&after[..n]);
        rest = &after[n + 2..];
    }
    out
}

/// One request over raw TCP, the connection closed by the server: the status, the headers
/// (lowercased), the body.
pub fn http(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> (u16, String, String) {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n");
    if let Some(b) = body {
        req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len()));
    }
    req.push_str("\r\n");
    if let Some(b) = body {
        req.push_str(b);
    }
    s.write_all(req.as_bytes()).unwrap();
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).unwrap();
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").expect("a header block");
    let status: u16 = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    let lower = head.to_ascii_lowercase();
    let body = if lower.contains("transfer-encoding: chunked") { dechunk(body) } else { body.to_string() };
    (status, lower, body)
}

/// A value computed in process as a client reads it once served: written as the server writes it
/// (the shortest text that round-trips) and parsed back as `get` parses an answer. serde_json's
/// default parser may land a 17-digit number one ULP off, the same way on both sides only when both
/// went through text, so a computed value is compared with a served one through this.
pub fn as_served(v: &Value) -> Value {
    serde_json::from_str(&v.to_string()).unwrap()
}

pub fn get(addr: SocketAddr, path: &str) -> (u16, Value) {
    let (status, _, body) = http(addr, "GET", path, None);
    (status, serde_json::from_str(&body).unwrap_or_else(|e| panic!("{path}: {e}: {body}")))
}

pub fn post(addr: SocketAddr, path: &str, body: &Value) -> (u16, Value) {
    let (status, _, text) = http(addr, "POST", path, Some(&body.to_string()));
    (status, serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}: {text}")))
}

/// A query value with the characters a node id carries escaped.
pub fn enc(s: &str) -> String {
    s.replace('%', "%25").replace('=', "%3D").replace(';', "%3B").replace('&', "%26").replace(' ', "%20")
}

/// An open server-sent event stream, read event by event.
pub struct Sse {
    stream: TcpStream,
    buf: String,
    pub status: u16,
    pub headers: String,
}

impl Sse {
    pub fn open(addr: SocketAddr, path: &str) -> Sse {
        let mut stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        stream.write_all(format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nAccept: text/event-stream\r\n\r\n").as_bytes()).unwrap();
        let mut sse = Sse { stream, buf: String::new(), status: 0, headers: String::new() };
        let started = Instant::now();
        while !sse.buf.contains("\r\n\r\n") && started.elapsed() < Duration::from_secs(10) {
            sse.fill();
        }
        let (head, rest) = {
            let (h, r) = sse.buf.split_once("\r\n\r\n").expect("a header block");
            (h.to_string(), r.to_string())
        };
        sse.status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        sse.headers = head.to_ascii_lowercase();
        sse.buf = rest;
        sse
    }

    /// Whether the server ends the stream (an end of file or a reset) within `within`, whatever
    /// it sends before.
    pub fn ends_within(&mut self, within: Duration) -> bool {
        let started = Instant::now();
        let mut chunk = [0u8; 65536];
        while started.elapsed() < within {
            match self.stream.read(&mut chunk) {
                Ok(0) => return true,
                Ok(_) => {}
                Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                Err(_) => return true,
            }
        }
        false
    }

    fn fill(&mut self) {
        let mut chunk = [0u8; 65536];
        if let Ok(n) = self.stream.read(&mut chunk)
            && n > 0
        {
            self.buf.push_str(&String::from_utf8_lossy(&chunk[..n]));
        }
    }

    /// The next event (its name and its data), within `within`; chunk framing and comments read
    /// past.
    pub fn next(&mut self, within: Duration) -> Option<(String, String)> {
        let started = Instant::now();
        loop {
            // Drop the chunked transfer's size lines: an event's lines never look like hex alone.
            let mut event = None;
            let text = self.buf.replace("\r\n", "\n");
            if let Some(end) = text.find("\n\n") {
                let block = &text[..end];
                let mut name = String::from("message");
                let mut data = String::new();
                let mut any = false;
                for line in block.lines() {
                    if let Some(v) = line.strip_prefix("event:") {
                        name = v.trim().to_string();
                        any = true;
                    } else if let Some(v) = line.strip_prefix("data:") {
                        if !data.is_empty() {
                            data.push('\n');
                        }
                        data.push_str(v.strip_prefix(' ').unwrap_or(v));
                        any = true;
                    }
                }
                self.buf = text[end + 2..].to_string();
                if any {
                    event = Some((name, data));
                } else {
                    continue;
                }
            }
            if event.is_some() {
                return event;
            }
            if started.elapsed() > within {
                return None;
            }
            self.fill();
        }
    }

    /// Events until one satisfies `done` (it included), within `within`.
    pub fn until(&mut self, within: Duration, mut done: impl FnMut(&str, &Value) -> bool) -> Option<Vec<(String, Value)>> {
        let started = Instant::now();
        let mut out = Vec::new();
        while started.elapsed() < within {
            if let Some((name, data)) = self.next(within.saturating_sub(started.elapsed())) {
                let v: Value = serde_json::from_str(&data).unwrap_or(Value::String(data));
                let hit = done(&name, &v);
                out.push((name, v));
                if hit {
                    return Some(out);
                }
            }
        }
        None
    }
}

/// An object of a snapshot by id.
pub fn object<'a>(snapshot: &'a Value, id: &str) -> &'a Value {
    snapshot["objects"].as_array().unwrap().iter().find(|o| o["id"] == id).unwrap_or_else(|| panic!("no object {id} in {snapshot}"))
}

/// Text drawn as the page wrote it, by the browser's computed style (`asWritten(el)`: its offences,
/// none for an element drawn as written): no text the stylesheet generates (before or after it, a
/// list item's marker); and of an element with words of its own, none drawn otherwise than
/// written: its case changed (by `text-transform` on it, on its first letter or on its first line),
/// its glyphs chosen otherwise (every `font-variant…` property the browser lists, and
/// `font-feature-settings`, on it and on its first letter and line, each `normal`), its letters
/// masked, set vertically (`writing-mode`), or reordered (a direction other than left to right, an
/// override of the text's own order, or any letter drawn left of the one before it on its line, read
/// by a range per letter), any letter drawn at no width, narrower than half its glyph's own advance
/// (measured apart in its font, at its drawn scale) or over the one before it, and its first
/// letter or first line painted, sized or spaced otherwise than the element. A font's own glyphs
/// for the same characters are the stated limit. Shared by the views' tests (the dashboards', the
/// Tables view's).
#[allow(dead_code)]
pub const AS_WRITTEN_FN: &str = r#"const asWritten = (el) => {
  const out = [];
  for (const pseudo of ["::before", "::after", "::marker"]) {
    const g = getComputedStyle(el, pseudo).content;
    if (g && g !== "none" && g !== "normal") out.push(`${pseudo}: text the stylesheet generates, ${g}`);
  }
  const c = getComputedStyle(el);
  if (c.display.includes("list-item") && (c.listStyleType !== "none" || c.listStyleImage !== "none")) out.push("a list item's marker");
  const texts = [...el.childNodes].filter((n) => n.nodeType === 3 && n.textContent.trim() !== "");
  if (!texts.length) return out;
  // Its first letter and its first line drawn as it is (the page writes no rule for either): their
  // paint, their size and their spacing its own.
  for (const pseudo of ["::first-letter", "::first-line"]) {
    const s = getComputedStyle(el, pseudo);
    for (const k of ["color", "-webkit-text-fill-color", "-webkit-text-stroke-color", "-webkit-text-stroke-width", "font-size", "font-weight", "font-style", "letter-spacing", "word-spacing", "visibility", "text-shadow"]) {
      if (s.getPropertyValue(k) !== c.getPropertyValue(k)) out.push(`its text${pseudo} drawn with another ${k}: ${s.getPropertyValue(k)}`);
    }
  }
  for (const pseudo of ["", "::first-letter", "::first-line"]) {
    const s = pseudo ? getComputedStyle(el, pseudo) : c;
    if (s.textTransform !== "none") out.push(`its text${pseudo} drawn ${s.textTransform}`);
    for (const k of [...s].filter((k) => k.startsWith("font-variant") || k === "font-feature-settings")) {
      if (s.getPropertyValue(k) !== "normal") out.push(`its glyphs${pseudo} chosen by ${k}: ${s.getPropertyValue(k)}`);
    }
  }
  if ((c.webkitTextSecurity ?? "none") !== "none") out.push(`its letters masked, ${c.webkitTextSecurity}`);
  if (c.writingMode !== "horizontal-tb") out.push(`its letters set ${c.writingMode}`);
  if (c.direction !== "ltr" || ["bidi-override", "isolate-override"].includes(c.unicodeBidi)) out.push(`its letters reordered, ${c.direction} ${c.unicodeBidi}`);
  // Its letters read where they are drawn: an element the browser renders no box for (an SVG
  // `title`, a hover text) draws none (whether a mark is shown at all is the shown census's).
  if (!el.getClientRects().length) return out;
  // Each letter's own advance in its font (measured apart, at the scale it is drawn at: an SVG text's
  // by its chart's matrix), so a letter squeezed onto its neighbours (a negative spacing, a length
  // forced on the text) is read as drawn narrower than its glyph.
  const pen = document.createElement("canvas").getContext("2d");
  pen.font = `${c.fontStyle} ${c.fontWeight} ${c.fontSize} ${c.fontFamily}`;
  const m = el instanceof SVGGraphicsElement ? el.getScreenCTM() : null;
  const k = m ? Math.hypot(m.a, m.b) : 1;
  const r = document.createRange();
  for (const n of texts) {
    let prev = null;
    for (let i = 0; i < n.length; i++) {
      const code = n.data.charCodeAt(i);
      const end = code >= 0xd800 && code <= 0xdbff ? i + 2 : i + 1;
      if (/\s/.test(n.data[i])) { i = end - 1; continue; }
      r.setStart(n, i);
      r.setEnd(n, Math.min(end, n.length));
      const b = r.getBoundingClientRect();
      const ch = n.data.slice(i, Math.min(end, n.length));
      i = end - 1;
      // A letter drawn at no width (a combining mark or a format character apart, which draw
      // none of their own), or over the one before it on its line by more than a quarter of the
      // narrower (kerning moves a letter a little, never onto its neighbour).
      if (!(b.width > 0)) {
        if (!/[\p{M}\p{Cf}]/u.test(ch)) { out.push(`its letter ${JSON.stringify(ch)} drawn at no width: ${JSON.stringify(n.data.slice(0, 30))}`); break; }
        continue;
      }
      const glyph = pen.measureText(ch).width * k;
      if (glyph > 0 && b.width < 0.5 * glyph) { out.push(`its letter ${JSON.stringify(ch)} drawn ${b.width.toFixed(2)} wide, its glyph ${glyph.toFixed(2)}: ${JSON.stringify(n.data.slice(0, 30))}`); break; }
      if (prev && Math.abs(b.top - prev.top) < 1 && b.left < prev.left - 0.5) { out.push(`its letters drawn out of their order at ${i}: ${JSON.stringify(n.data.slice(0, 30))}`); break; }
      if (prev && Math.abs(b.top - prev.top) < 1 && prev.right - b.left > 0.25 * Math.min(b.width, prev.width) + 0.5) { out.push(`its letters drawn over one another at ${i}: ${JSON.stringify(n.data.slice(0, 30))}`); break; }
      prev = b;
    }
  }
  return out;
};"#;

/// What stops a mark painting that its box, its visibility and its paint's colour do not say, for the
/// views' tests' readings of a mark shown: `hiddenBy(el)` a clip (`clip-path`, `clip`), a filter or a
/// mask on it or on any element around it (the page writes none), or its own box cutting what it
/// holds (its overflow hidden, its content past it: an ellipsis among it); `strokeDrawn(c)` false for a
/// stroke whose dashes are all of no length; `referenced(p)` whether a paint by reference
/// (`url(#…)`) reaches a gradient or a pattern, or falls back to a colour.
#[allow(dead_code)]
pub const HIDDEN_BY_FN: &str = r#"const hiddenBy = (el) => {
  for (let e = el; e && e.nodeType === 1; e = e.parentElement) {
    const s = getComputedStyle(e), at = e.tagName.toLowerCase();
    if ((s.clipPath || "none") !== "none") return `clipped by clip-path on ${at}`;
    if ((s.clip || "auto") !== "auto") return `clipped by clip on ${at}`;
    if ((s.filter || "none") !== "none") return `filtered on ${at}`;
    if ([s.maskImage, s.webkitMaskImage].some((v) => v && v !== "none")) return `masked on ${at}`;
  }
  if (!(el instanceof SVGElement)) {
    const s = getComputedStyle(el);
    if ([s.overflowX, s.overflowY].some((v) => v === "hidden" || v === "clip") && (el.scrollWidth > el.clientWidth + 1 || el.scrollHeight > el.clientHeight + 1)) return "cut by its own box";
  }
  return null;
};
const strokeDrawn = (c) => {
  if (!c.strokeDasharray || c.strokeDasharray === "none") return true;
  const d = c.strokeDasharray.split(/[\s,]+/).map(parseFloat).filter((x) => !Number.isNaN(x));
  return (d.length % 2 ? d.concat(d) : d).some((x, i) => i % 2 === 0 && x > 0);
};
const referenced = (p) => {
  const m = /^url\(\s*["']?[^"')#]*#([^"')]+)["']?\s*\)\s*(.*)$/.exec(p);
  if (!m) return false;
  const t = document.getElementById(decodeURIComponent(m[1]));
  if (t && ["lineargradient", "radialgradient", "pattern"].includes(t.tagName.toLowerCase())) return true;
  return m[2] !== "" && m[2] !== "none";
};"#;

/// The reference's drawing (its loop, infeed, outlets and parcels, by the same ids) as Inkscape 1.3
/// saves a drawing (Q25 decided: layouts are drawn in Inkscape): the XML declaration and a comment,
/// the SVG namespace both unprefixed and as `svg:`, Inkscape's own namespaces, its editor's state
/// (`sodipodi:namedview` with a grid, a page and a guide; a path effect and a perspective in `defs`),
/// the metadata block in `metadata`, layers (one locked, one moved, one hidden), every paint in a
/// `style`, lines as paths with their node types, an arc as a path, a group turned by a matrix, a
/// text with its line span; the markers writing `display:inline` (as Inkscape writes on an object
/// hidden and shown again), and the loop speed's object on a text whose line span holds a styled run
/// (as Inkscape writes a run styled apart inside a line). Written by hand to Inkscape's format: no
/// file saved by Inkscape has gone through the reader or the page (the limit C-048 and C-051 state).
pub const INKSCAPE_LOOP: &str = r##"<?xml version="1.0" encoding="UTF-8" standalone="no"?>
<!-- Created with Inkscape (http://www.inkscape.org/) -->

<svg
   width="105.83333mm"
   height="58.208332mm"
   viewBox="0 0 400 220"
   version="1.1"
   id="svg1"
   inkscape:version="1.3.2 (091e20e, 2023-11-25, custom)"
   sodipodi:docname="loop.svg"
   xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape"
   xmlns:sodipodi="http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd"
   xmlns="http://www.w3.org/2000/svg"
   xmlns:svg="http://www.w3.org/2000/svg"
   xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
   xmlns:cc="http://creativecommons.org/ns#"
   xmlns:dc="http://purl.org/dc/elements/1.1/">
  <sodipodi:namedview
     id="namedview1"
     pagecolor="#ffffff"
     bordercolor="#000000"
     borderopacity="0.25"
     inkscape:showpageshadow="2"
     inkscape:pageopacity="0.0"
     inkscape:pagecheckerboard="0"
     inkscape:deskcolor="#d1d1d1"
     inkscape:document-units="mm"
     showgrid="true"
     inkscape:zoom="0.73"
     inkscape:cx="396"
     inkscape:cy="561"
     inkscape:window-width="1920"
     inkscape:window-height="1017"
     inkscape:window-x="-8"
     inkscape:window-y="-8"
     inkscape:window-maximized="1"
     inkscape:current-layer="layer1">
    <inkscape:grid
       id="grid1"
       units="mm"
       originx="0"
       originy="0"
       spacingx="1"
       spacingy="1"
       empcolor="#0099e5"
       empopacity="0.30196078"
       color="#0099e5"
       opacity="0.14901961"
       empspacing="5"
       dotted="false"
       gridanglex="30"
       gridanglez="30"
       visible="true" />
    <inkscape:page
       x="0"
       y="0"
       width="400"
       height="220"
       id="page1"
       margin="0"
       bleed="0" />
    <sodipodi:guide
       position="20,124"
       orientation="0,-1"
       id="guide1"
       inkscape:locked="false" />
  </sodipodi:namedview>
  <defs
     id="defs1">
    <inkscape:path-effect
       effect="spiro"
       id="path-effect1"
       is_visible="true"
       lpeversion="1" />
    <inkscape:perspective
       sodipodi:type="inkscape:persp3d"
       inkscape:vp_x="0 : 110 : 1"
       inkscape:vp_y="0 : 1000 : 0"
       inkscape:vp_z="400 : 110 : 1"
       inkscape:persp3d-origin="200 : 73.333333 : 1"
       id="perspective1" />
    <linearGradient
       id="linearGradient1"
       inkscape:collect="always">
      <stop
         style="stop-color:#26262a;stop-opacity:1"
         offset="0"
         id="stop1" />
      <stop
         style="stop-color:#3a3a40;stop-opacity:1"
         offset="1"
         id="stop2" />
    </linearGradient>
  </defs>
  <metadata
     id="metadata1">
    <rdf:RDF>
      <cc:Work
         rdf:about="">
        <dc:format>image/svg+xml</dc:format>
        <dc:type
           rdf:resource="http://purl.org/dc/dcmitype/StillImage" />
        <dc:title>Loop</dc:title>
        <dc:creator>
          <cc:Agent>
            <dc:title>Plant engineering</dc:title>
          </cc:Agent>
        </dc:creator>
      </cc:Work>
    </rdf:RDF>
  </metadata>
  <g
     inkscape:label="Frame"
     inkscape:groupmode="layer"
     id="layer3"
     sodipodi:insensitive="true"
     style="display:inline">
    <rect
       style="fill:none;stroke:#3a3a40;stroke-width:0.5;stroke-dasharray:2,2"
       id="rect9"
       width="396"
       height="216"
       x="2"
       y="2" />
  </g>
  <g
     inkscape:label="Plant"
     inkscape:groupmode="layer"
     id="layer1"
     transform="translate(12.5,-4)"
     style="display:inline">
    <path
       style="fill:none;stroke:#26262a;stroke-width:14;stroke-linecap:butt;stroke-linejoin:miter;stroke-opacity:1"
       d="M 80,40 H 320 A 70,70 0 0 1 320,180 H 80 A 70,70 0 0 1 80,40 Z"
       id="loop"
       inkscape:label="Loop"
       sodipodi:nodetypes="ccccc" />
    <rect
       style="fill:#cccccc;stroke:#000000;stroke-width:0.264583"
       id="infeed"
       width="40"
       height="28"
       x="20"
       y="96"
       ry="2"
       inkscape:label="Infeed" />
    <g
       id="g1"
       inkscape:label="Outlets">
      <rect
         style="fill:#cccccc;stroke:#000000;stroke-width:0.264583"
         id="outlet-1"
         width="24"
         height="28"
         x="130"
         y="4" />
      <rect
         style="fill:#cccccc;stroke:#000000;stroke-width:0.264583"
         id="outlet-2"
         width="24"
         height="28"
         x="190"
         y="4" />
      <rect
         style="fill:#cccccc;stroke:#000000;stroke-width:0.264583"
         id="outlet-3"
         width="24"
         height="28"
         x="250"
         y="4" />
    </g>
    <circle
       style="display:inline;fill:#ffffff;stroke:#000000;stroke-width:0.264583"
       id="parcel-1"
       cx="150"
       cy="60"
       r="5" />
    <circle
       style="display:inline;fill:#ffffff;stroke:#000000;stroke-width:0.264583"
       id="parcel-2"
       cx="250"
       cy="160"
       r="5" />
    <g
       id="g2"
       transform="matrix(0.8660254,0.5,-0.5,0.8660254,90.5,-36)"
       inkscape:label="Turned">
      <circle
         style="fill:url(#linearGradient1);stroke:#000000;stroke-width:0.264583"
         id="circle1"
         cx="200"
         cy="110"
         r="6" />
    </g>
    <path
       style="fill:none;stroke:#3a3a40;stroke-width:1"
       id="path2"
       sodipodi:type="arc"
       sodipodi:cx="200"
       sodipodi:cy="110"
       sodipodi:rx="12"
       sodipodi:ry="12"
       sodipodi:start="0"
       sodipodi:end="3.1415927"
       sodipodi:arc-type="arc"
       d="M 212,110 A 12,12 0 0 1 200,122 12,12 0 0 1 188,110"
       sodipodi:open="true" />
    <path
       style="fill:none;stroke:#3a3a40;stroke-width:2;stroke-linecap:butt;stroke-linejoin:miter;stroke-opacity:1"
       d="M 60,110 H 80"
       id="path1"
       sodipodi:nodetypes="cc" />
    <text
       xml:space="preserve"
       style="font-size:10.5833px;font-family:sans-serif;-inkscape-font-specification:sans-serif;fill:#e8e8ec;stroke-width:0.264583"
       x="20"
       y="140"
       id="text1"><tspan
         sodipodi:role="line"
         id="tspan1"
         style="stroke-width:0.264583"
         x="20"
         y="140">Infeed</tspan></text>
    <text
       xml:space="preserve"
       style="font-size:8px;font-family:sans-serif;-inkscape-font-specification:sans-serif;fill:#e8e8ec;stroke-width:0.264583"
       x="110"
       y="118"
       id="speed"><tspan
         sodipodi:role="line"
         id="tspan3"
         style="stroke-width:0.264583"
         x="110"
         y="118">Spe<tspan
           style="fill:#ff0000;stroke:#00ff00;stroke-width:3"
           id="tspan4">ed</tspan></tspan></text>
  </g>
  <g
     inkscape:label="Notes"
     inkscape:groupmode="layer"
     id="layer2"
     style="display:none"
     sodipodi:insensitive="true">
    <text
       xml:space="preserve"
       style="font-size:8px;font-family:sans-serif;-inkscape-font-specification:sans-serif;fill:#ff8f8f"
       x="20"
       y="200"
       id="text2"><tspan
         sodipodi:role="line"
         id="tspan2"
         x="20"
         y="200">check the outlet 3 sensor</tspan></text>
  </g>
</svg>
"##;
