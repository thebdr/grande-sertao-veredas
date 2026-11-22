//! The driver every shell test uses: the shell started with no display and Spix's test server on
//! a port of its own, and a small XML-RPC client that speaks to it.
//!
//! Spix answers an action (a click, typed text) before the action has landed in the interface: a
//! read straight after it can see the state before. So a test never reads once; it waits for the
//! value it expects ([`Shell::wait_for`]), and a value that never comes fails with the last one
//! read.

#![allow(dead_code)]

// The build script links the shell's generated C++ into every target of the package, a test's
// included, and that code calls cxx-qt's initialisers and the bridge's Rust side: the test links
// those crates too.
extern crate cxx_qt;
extern crate cxx_qt_lib;
extern crate oficina_spike_shell;

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A value as XML-RPC carries it.
#[derive(Clone, Debug, PartialEq)]
pub enum X {
    Str(String),
    Int(i64),
    Bool(bool),
    Double(f64),
    Array(Vec<X>),
    Nil,
}

impl X {
    pub fn str(&self) -> &str {
        match self {
            X::Str(s) => s,
            other => panic!("not a string: {other:?}"),
        }
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find(';') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let entity = &rest[i + 1..i + j];
        match entity {
            "amp" => out.push('&'),
            "lt" => out.push('<'),
            "gt" => out.push('>'),
            "quot" => out.push('"'),
            "apos" => out.push('\''),
            e if e.starts_with("#x") => out.extend(u32::from_str_radix(&e[2..], 16).ok().and_then(char::from_u32)),
            e if e.starts_with('#') => out.extend(e[1..].parse().ok().and_then(char::from_u32)),
            e => {
                out.push('&');
                out.push_str(e);
                out.push(';');
            }
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    out
}

fn encode(v: &X) -> String {
    match v {
        X::Str(s) => format!("<value><string>{}</string></value>", escape(s)),
        X::Int(n) => format!("<value><int>{n}</int></value>"),
        X::Bool(b) => format!("<value><boolean>{}</boolean></value>", u8::from(*b)),
        X::Double(d) => format!("<value><double>{d}</double></value>"),
        X::Array(a) => format!("<value><array><data>{}</data></array></value>", a.iter().map(encode).collect::<String>()),
        X::Nil => "<value><nil/></value>".to_string(),
    }
}

/// The text between `<tag>` and its `</tag>` at the start of `s` (after spaces), and what follows.
fn element<'a>(s: &'a str, tag: &str) -> Option<(&'a str, &'a str)> {
    let s = s.trim_start();
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let body = s.strip_prefix(&open)?;
    // Values nest only through arrays; a value's closing tag is found by counting.
    let mut depth = 0usize;
    let mut i = 0;
    while i < body.len() {
        if body[i..].starts_with(&open) {
            depth += 1;
            i += open.len();
        } else if body[i..].starts_with(&close) {
            if depth == 0 {
                return Some((&body[..i], &body[i + close.len()..]));
            }
            depth -= 1;
            i += close.len();
        } else {
            i += body[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
    None
}

fn decode(value_body: &str) -> X {
    let b = value_body.trim();
    if b.is_empty() {
        return X::Str(String::new());
    }
    for tag in ["string", "int", "i4", "i8", "boolean", "double", "array"] {
        if let Some((inner, _)) = element(b, tag) {
            return match tag {
                "string" => X::Str(unescape(inner)),
                "int" | "i4" | "i8" => X::Int(inner.trim().parse().expect("an integer")),
                "boolean" => X::Bool(inner.trim() == "1"),
                "double" => X::Double(inner.trim().parse().expect("a double")),
                _ => {
                    let mut items = Vec::new();
                    let (mut data, _) = element(inner, "data").unwrap_or(("", ""));
                    while let Some((v, rest)) = element(data, "value") {
                        items.push(decode(v));
                        data = rest;
                    }
                    X::Array(items)
                }
            };
        }
    }
    if b.starts_with("<nil") {
        return X::Nil;
    }
    // A value with no type is a string, every character of it, its spaces and line ends included.
    X::Str(unescape(value_body))
}

/// Qt's key codes and Spix's modifier for the keys the tests press.
pub const KEY_A: i64 = 0x41;
pub const KEY_RETURN: i64 = 0x0100_0004;
pub const CONTROL: i64 = 2;

/// The shell, running with no display, and the port its test server listens on.
pub struct Shell {
    child: Child,
    port: u16,
    pub log: std::path::PathBuf,
}

/// A port nobody listens on now.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").expect("a port").local_addr().expect("its address").port()
}

impl Shell {
    /// Start the shell with `args` on Qt's `offscreen` platform, or on the one
    /// `OFICINA_TEST_PLATFORM` names, and wait until its test server answers.
    pub fn start(args: &[&str], envs: &[(&str, &str)]) -> Shell {
        let port = free_port();
        let log = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("oficina-spike-shell-{port}.log"));
        let out = std::fs::File::create(&log).expect("the log");
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_oficina-spike-shell"));
        // Qt's platform: `offscreen` (no display) unless the run names another, `xcb` for X11.
        let platform = std::env::var("OFICINA_TEST_PLATFORM").unwrap_or_else(|_| "offscreen".to_string());
        cmd.args(args)
            .env("QT_QPA_PLATFORM", platform)
            .env("OFICINA_SPIX_PORT", port.to_string())
            .stdin(Stdio::null())
            .stdout(out.try_clone().expect("the log"))
            .stderr(out);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let child = cmd.spawn().expect("the shell starts");
        let shell = Shell { child, port, log };
        let until = Instant::now() + Duration::from_secs(30);
        loop {
            if shell.try_call("wait", &[X::Int(1)]).is_ok() {
                return shell;
            }
            assert!(Instant::now() < until, "the shell's test server never answered; its log:\n{}", shell.log_text());
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The shell's resident memory in KiB, as the kernel counts it (`VmRSS`).
    #[cfg(target_os = "linux")]
    pub fn resident_kib(&self) -> u64 {
        let status = std::fs::read_to_string(format!("/proc/{}/status", self.child.id())).expect("the process's status");
        status
            .lines()
            .find_map(|l| l.strip_prefix("VmRSS:"))
            .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
            .expect("VmRSS")
    }

    pub fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    pub fn try_call(&self, method: &str, params: &[X]) -> Result<X, String> {
        let body = format!(
            "<?xml version=\"1.0\"?><methodCall><methodName>{method}</methodName><params>{}</params></methodCall>",
            params.iter().map(|p| format!("<param>{}</param>", encode(p))).collect::<String>()
        );
        let mut s = TcpStream::connect(("127.0.0.1", self.port)).map_err(|e| e.to_string())?;
        s.set_read_timeout(Some(Duration::from_secs(30))).map_err(|e| e.to_string())?;
        write!(
            s,
            "POST /RPC2 HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .map_err(|e| e.to_string())?;
        let mut raw = Vec::new();
        // Read the head, then exactly the body its length announces.
        let mut buf = [0u8; 4096];
        let (head_end, length) = loop {
            let n = s.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("the server closed before answering".into());
            }
            raw.extend_from_slice(&buf[..n]);
            if let Some(i) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&raw[..i]).to_ascii_lowercase();
                let length = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>()));
                break (i + 4, length.and_then(Result::ok));
            }
        };
        loop {
            if length.is_some_and(|len| raw.len() >= head_end + len) {
                break;
            }
            let n = s.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n]);
        }
        let text = String::from_utf8_lossy(&raw[head_end..]).into_owned();
        if text.contains("<fault>") {
            return Err(text);
        }
        let start = text.find("<param>").ok_or_else(|| format!("no answer in {text}"))?;
        let (param, _) = element(&text[start..], "param").ok_or_else(|| format!("no param in {text}"))?;
        let (value, _) = element(param, "value").ok_or_else(|| format!("no value in {text}"))?;
        Ok(decode(value))
    }

    pub fn call(&self, method: &str, params: &[X]) -> X {
        self.try_call(method, params).unwrap_or_else(|e| panic!("{method}: {e}\nthe shell's log:\n{}", self.log_text()))
    }

    pub fn click(&self, path: &str) {
        self.call("mouseClick", &[X::Str(path.into())]);
    }

    pub fn type_text(&self, path: &str, text: &str) {
        self.call("inputText", &[X::Str(path.into()), X::Str(text.into())]);
    }

    pub fn property(&self, path: &str, name: &str) -> String {
        self.call("getStringProperty", &[X::Str(path.into()), X::Str(name.into())]).str().to_string()
    }

    pub fn set_property(&self, path: &str, name: &str, value: &str) {
        self.call("setStringProperty", &[X::Str(path.into()), X::Str(name.into()), X::Str(value.into())]);
    }

    pub fn invoke(&self, path: &str, method: &str, args: &[X]) -> X {
        self.call("invokeMethod", &[X::Str(path.into()), X::Str(method.into()), X::Array(args.to_vec())])
    }

    pub fn visible(&self, path: &str) -> bool {
        self.call("existsAndVisible", &[X::Str(path.into())]) == X::Bool(true)
    }

    /// Wait until the property reads `expected`, and fail with the last value read if it never does.
    pub fn wait_for(&self, path: &str, name: &str, expected: &str) {
        self.wait_until(path, name, Duration::from_secs(10), |v| v == expected, expected);
    }

    pub fn wait_until(&self, path: &str, name: &str, within: Duration, ok: impl Fn(&str) -> bool, what: &str) -> String {
        let until = Instant::now() + within;
        loop {
            let v = self.try_call("getStringProperty", &[X::Str(path.into()), X::Str(name.into())]);
            if let Ok(X::Str(s)) = &v
                && ok(s)
            {
                return s.clone();
            }
            assert!(Instant::now() < until, "{path}.{name}: waited for {what:?}, last read {v:?}\nthe shell's log:\n{}", self.log_text());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn errors(&self) -> Vec<String> {
        match self.call("getErrors", &[]) {
            X::Array(a) => a.into_iter().map(|e| e.str().to_string()).collect(),
            other => panic!("errors: {other:?}"),
        }
    }

    /// The labels of the rows `row0`, `row1`, ... as the tree draws them, up to the first row
    /// not drawn.
    pub fn rows(&self) -> Vec<String> {
        (0..).map_while(|i| {
            let path = format!("oficina/row{i}");
            self.visible(&path).then(|| self.property(&path, "label"))
        }).collect()
    }

    /// Wait until the tree draws exactly `expected`, and fail with the last rows read.
    pub fn wait_for_rows(&self, expected: &[&str]) {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            let rows = self.rows();
            if rows == expected {
                return;
            }
            assert!(Instant::now() < until, "the tree's rows: waited for {expected:#?}\nlast read {rows:#?}\nthe shell's log:\n{}", self.log_text());
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The path of the tree's first row whose label begins with `prefix`, waiting for it.
    pub fn row_of(&self, prefix: &str) -> String {
        let until = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(i) = self.rows().iter().position(|r| r.starts_with(prefix)) {
                return format!("oficina/row{i}");
            }
            assert!(Instant::now() < until, "no row begins with {prefix:?}: {:#?}", self.rows());
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Press a key with modifiers (Spix's: 1 Shift, 2 Control, 4 Alt) at the item with focus.
    pub fn key(&self, path: &str, key: i64, modifiers: i64) {
        self.call("enterKey", &[X::Str(path.into()), X::Int(key), X::Int(modifiers)]);
    }

    /// Replace a text field's whole text as a person does: click it, select all, type, Return.
    pub fn retype(&self, path: &str, text: &str) {
        self.click(path);
        self.wait_for(path, "activeFocus", "true");
        self.key(path, KEY_A, CONTROL);
        self.type_text(path, text);
        self.key(path, KEY_RETURN, 0);
    }

    pub fn screenshot(&self, path: &str, file: &Path) {
        self.call("takeScreenshot", &[X::Str(path.into()), X::Str(file.display().to_string())]);
    }
}

impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.try_call("quit", &[]);
        let until = Instant::now() + Duration::from_secs(10);
        while Instant::now() < until {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
