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
    let text = "format = \"veredas-bench\"\nversion = 1\nseed = 5\n\n[[opc]]\nname = \"plc\"\nport = 0\ntags = [\n  { id = \"speed\", type = \"float\", value = \"1.5\" },\n  { id = \"p1\", type = \"float\", value = \"25\" },\n  { id = \"p2\", type = \"float\", value = \"75\" },\n  { id = \"fill1\", type = \"float\", value = \"35\" },\n  { id = \"fill2\", type = \"float\", value = \"85\" },\n  { id = \"fill3\", type = \"float\", value = \"100\" },\n  { id = \"infeed\", type = \"int\", value = \"1\" },\n  { id = \"release3\", type = \"int\", value = \"0\", access = \"read-write\" },\n]\n";
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
