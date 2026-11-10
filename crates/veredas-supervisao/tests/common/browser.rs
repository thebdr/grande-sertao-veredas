//! A headless Chromium (Edge on Windows, Chrome or Chromium elsewhere; `VEREDAS_BROWSER` names
//! another) run on a page and driven over the DevTools protocol, so a test runs the page as a
//! person's browser does and reads what it drew. The protocol is spoken over a WebSocket written
//! here in a few lines (text frames, masked out, unmasked in), so no crate is added for it.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// Where a Chromium is looked for, in order, after `VEREDAS_BROWSER`.
pub const CANDIDATES: &[&str] = &[
    "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Microsoft/Edge/Application/msedge.exe",
    "C:/Program Files/Google/Chrome/Application/chrome.exe",
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
    "/usr/bin/microsoft-edge",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
];

/// The browser the suite runs pages in: a missing one is a failure, never a skip.
pub fn executable() -> PathBuf {
    if let Some(p) = std::env::var_os("VEREDAS_BROWSER") {
        return PathBuf::from(p);
    }
    CANDIDATES.iter().map(PathBuf::from).find(|p| p.is_file()).unwrap_or_else(|| panic!("no Chromium browser found at {CANDIDATES:?}: set VEREDAS_BROWSER to one"))
}

pub struct Browser {
    child: Child,
    ws: TcpStream,
    next: u64,
    /// The protocol's events received while waiting for answers, in order.
    events: Vec<Value>,
    _profile: tempfile::TempDir,
}

/// One HTTP GET to the browser's own endpoint, its body as JSON.
fn devtools_get(addr: SocketAddr, path: &str) -> Value {
    let mut s = TcpStream::connect(addr).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(s, "GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").unwrap();
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut b).unwrap();
        head.push(b[0]);
    }
    let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
    let len: usize = head.lines().find_map(|l| l.strip_prefix("content-length:")).map(|v| v.trim().parse().unwrap()).unwrap();
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// One text frame, masked as a client's are.
fn frame(text: &str) -> Vec<u8> {
    let p = text.as_bytes();
    let mask = [0x37u8, 0xfa, 0x21, 0x3d];
    let mut f = vec![0x81u8];
    match p.len() {
        n if n < 126 => f.push(0x80 | n as u8),
        n if n < 65536 => {
            f.push(0x80 | 126);
            f.extend((n as u16).to_be_bytes());
        }
        n => {
            f.push(0x80 | 127);
            f.extend((n as u64).to_be_bytes());
        }
    }
    f.extend(mask);
    f.extend(p.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    f
}

impl Browser {
    /// The browser started headless on `url`, at a desktop's size, with a profile of its own.
    pub fn open(url: &str) -> Browser {
        Browser::start(url)
    }

    /// The same, every request the page makes watched from its first (`requests`): the browser
    /// opens a blank page, turns the protocol's network events on, then goes to `url`.
    pub fn watched(url: &str) -> Browser {
        let mut b = Browser::start("about:blank");
        b.call("Network.enable", json!({}));
        b.call("Page.navigate", json!({ "url": url }));
        b
    }

    /// Every request the page made since it was watched, as (URL, the protocol's resource type:
    /// Document, Script, Fetch, EventSource, Image, Ping, ...), streams and other origins included;
    /// a WebSocket, which the protocol reports apart, as (its URL, "WebSocket").
    pub fn requests(&mut self) -> Vec<(String, String)> {
        // An answer comes after every event sent before it: one round trip reads them all.
        self.eval("0");
        self.events
            .iter()
            .filter_map(|e| match e["method"].as_str() {
                Some("Network.requestWillBeSent") => Some((e["params"]["request"]["url"].as_str().unwrap_or("").to_string(), e["params"]["type"].as_str().unwrap_or("").to_string())),
                Some("Network.webSocketCreated") => Some((e["params"]["url"].as_str().unwrap_or("").to_string(), "WebSocket".to_string())),
                _ => None,
            })
            .collect()
    }

    /// Every POST the page made since it was watched, as (URL, the body as sent).
    pub fn posts(&mut self) -> Vec<(String, String)> {
        self.eval("0");
        self.events
            .iter()
            .filter(|e| e["method"] == "Network.requestWillBeSent" && e["params"]["request"]["method"] == "POST")
            .map(|e| (e["params"]["request"]["url"].as_str().unwrap_or("").to_string(), e["params"]["request"]["postData"].as_str().unwrap_or("").to_string()))
            .collect()
    }

    fn start(url: &str) -> Browser {
        let profile = tempfile::tempdir().unwrap();
        let child = Command::new(executable())
            .args(["--headless=new", "--disable-gpu", "--no-sandbox", "--no-first-run", "--no-default-browser-check", "--disable-extensions", "--remote-debugging-port=0", "--window-size=1440,900"])
            .arg(format!("--user-data-dir={}", profile.path().display()))
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        // The port the browser picked, written in its profile once it listens.
        let file = profile.path().join("DevToolsActivePort");
        let deadline = Instant::now() + Duration::from_secs(30);
        let port: u16 = loop {
            if let Some(p) = std::fs::read_to_string(&file).ok().and_then(|t| t.lines().next().and_then(|l| l.trim().parse().ok())) {
                break p;
            }
            assert!(Instant::now() < deadline, "the browser never opened its debugging port");
            std::thread::sleep(Duration::from_millis(100));
        };
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let page = loop {
            let list = devtools_get(addr, "/json/list");
            if let Some(ws) = list.as_array().and_then(|a| a.iter().find(|t| t["type"] == "page")).and_then(|t| t["webSocketDebuggerUrl"].as_str()) {
                break ws.split_once(&format!("{port}")).unwrap().1.to_string();
            }
            assert!(Instant::now() < deadline, "the browser never opened its page");
            std::thread::sleep(Duration::from_millis(100));
        };
        let mut ws = TcpStream::connect(addr).unwrap();
        ws.set_read_timeout(Some(Duration::from_secs(30))).unwrap();
        write!(ws, "GET {page} HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n").unwrap();
        let mut head = Vec::new();
        let mut b = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            ws.read_exact(&mut b).unwrap();
            head.push(b[0]);
        }
        assert!(head.starts_with(b"HTTP/1.1 101"), "{}", String::from_utf8_lossy(&head));
        Browser { child, ws, next: 0, events: Vec::new(), _profile: profile }
    }

    fn send(&mut self, text: &str) {
        let f = frame(text);
        self.ws.write_all(&f).unwrap();
    }

    fn recv(&mut self) -> String {
        let mut msg = Vec::new();
        loop {
            let mut h = [0u8; 2];
            self.ws.read_exact(&mut h).unwrap();
            let (fin, op) = (h[0] & 0x80 != 0, h[0] & 0x0f);
            let mut n = (h[1] & 0x7f) as u64;
            if n == 126 {
                let mut b = [0u8; 2];
                self.ws.read_exact(&mut b).unwrap();
                n = u16::from_be_bytes(b) as u64;
            } else if n == 127 {
                let mut b = [0u8; 8];
                self.ws.read_exact(&mut b).unwrap();
                n = u64::from_be_bytes(b);
            }
            let mut key = [0u8; 4];
            if h[1] & 0x80 != 0 {
                self.ws.read_exact(&mut key).unwrap();
            }
            let mut p = vec![0u8; n as usize];
            self.ws.read_exact(&mut p).unwrap();
            if h[1] & 0x80 != 0 {
                p.iter_mut().enumerate().for_each(|(i, b)| *b ^= key[i % 4]);
            }
            match op {
                0x8 => panic!("the browser closed the protocol's connection"),
                0x9 | 0xA => continue,
                _ => {
                    msg.extend(p);
                    if fin {
                        return String::from_utf8(msg).unwrap();
                    }
                }
            }
        }
    }

    /// A protocol call, answered by its result (events in between are passed over).
    pub fn call(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        self.send(&json!({ "id": id, "method": method, "params": params }).to_string());
        loop {
            let v: Value = serde_json::from_str(&self.recv()).unwrap();
            if v["id"] == id {
                assert!(v.get("error").is_none(), "{method}: {}", v["error"]);
                return v["result"].clone();
            }
            if v.get("method").is_some() && v["method"] != "Network.eventSourceMessageReceived" && v["method"] != "Network.dataReceived" {
                self.events.push(v);
            }
        }
    }

    /// An expression evaluated in the page (a promise awaited), its value as JSON.
    pub fn eval(&mut self, js: &str) -> Value {
        let r = self.call("Runtime.evaluate", json!({ "expression": js, "returnByValue": true, "awaitPromise": true }));
        assert!(r.get("exceptionDetails").is_none(), "{js}: {}", r["exceptionDetails"]);
        r["result"]["value"].clone()
    }

    /// The expression's value once it is truthy, within `within`.
    pub fn wait_for(&mut self, js: &str, within: Duration) -> Value {
        let deadline = Instant::now() + within;
        loop {
            let v = self.eval(js);
            if !(v.is_null() || v == json!(false) || v == json!("") || v == json!(0)) {
                return v;
            }
            assert!(Instant::now() < deadline, "waited {within:?} for `{js}`, last {v}");
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

impl Drop for Browser {
    fn drop(&mut self) {
        // Closed through the protocol, so its helper processes end with it; killed if it lingers.
        self.next += 1;
        let close = frame(&json!({ "id": self.next, "method": "Browser.close" }).to_string());
        let _ = self.ws.write_all(&close);
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
