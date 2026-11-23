//! A language-server client on Oficina's side (the spike's point 4; the review's entry 28: Monaco
//! has no client of its own): rust-analyzer started as a process and spoken to over its standard
//! streams, the messages framed and read here, what it says turned into what the editor draws.
//!
//! Opening a project runs nothing of it (the review's entry 32, not ruled; the spike measures it):
//! rust-analyzer is given the project as an inline description of its crates, so it never asks cargo
//! about the folder, which would honour the folder's `.cargo/config.toml` and its toolchain file;
//! build scripts and procedural macros are off; the server is the toolchain's own binary, not
//! rustup's proxy, which would honour a `rust-toolchain.toml` found where it starts; and it starts
//! in a folder of Oficina's, not the project's.

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

use serde_json::{Value, json};
use thiserror::Error;

/// A file path as the protocol names it. On Windows `C:\a\b` (or the `\\?\C:\a\b` that
/// `canonicalize` gives) is `file:///C:/a/b`.
pub fn uri_of(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = if cfg!(windows) {
        let plain = text.strip_prefix(r"\\?\").unwrap_or(&text);
        format!("/{}", plain.replace('\\', "/"))
    } else {
        text.into_owned()
    };
    let mut out = String::from("file://");
    for (i, b) in text.bytes().enumerate() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            // A drive's colon, `/C:`, as Windows paths begin.
            b':' if cfg!(windows) && i == 2 => out.push(':'),
            b => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// One message framed as the protocol frames it: its length, a blank line, the JSON.
pub fn frame(message: &Value) -> Vec<u8> {
    let body = message.to_string();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum FrameError {
    #[error("a header without `Content-Length`: {0:?}")]
    NoLength(String),
    #[error("a body that is not JSON: {0}")]
    NotJson(String),
}

/// The messages in a stream of bytes, as they come, in pieces of any size.
#[derive(Debug, Default)]
pub struct Unframer {
    buf: Vec<u8>,
}

impl Unframer {
    /// Take `bytes` and return every message they complete.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Value>, FrameError> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        loop {
            let Some(end) = self.buf.windows(4).position(|w| w == b"\r\n\r\n") else { return Ok(out) };
            let head = String::from_utf8_lossy(&self.buf[..end]).into_owned();
            let length = head
                .split("\r\n")
                .find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length")).map(|(_, v)| v.trim().parse::<usize>()))
                .and_then(Result::ok)
                .ok_or_else(|| FrameError::NoLength(head.clone()))?;
            let start = end + 4;
            if self.buf.len() < start + length {
                return Ok(out);
            }
            let body: Vec<u8> = self.buf.drain(..start + length).skip(start).collect();
            out.push(serde_json::from_slice(&body).map_err(|e| FrameError::NotJson(e.to_string()))?);
        }
    }
}

/// An ORCL project as rust-analyzer's inline project description: the program's crate, depending
/// on `orcl-std`, and the toolchain's own standard library. Nothing in it asks cargo anything.
pub fn orcl_project(program: &Path, orcl_std: &Path, sysroot: &Path) -> Value {
    json!({
        "sysroot": sysroot,
        "sysroot_src": sysroot.join("lib/rustlib/src/rust/library"),
        "crates": [
            {
                "display_name": "program",
                "root_module": program,
                "edition": "2024",
                "deps": [{ "crate": 1, "name": "orcl_std" }],
                "is_workspace_member": true,
                "cfg": []
            },
            {
                "display_name": "orcl_std",
                "root_module": orcl_std,
                "edition": "2024",
                "deps": [],
                "is_workspace_member": false,
                "cfg": []
            }
        ]
    })
}

/// rust-analyzer's settings: the project described inline, nothing built, nothing checked by cargo,
/// its own diagnostics on, the experimental ones among them (an unknown method or name is one).
///
/// Not `cargo.autoreload: false`: with it, rust-analyzer 1.100 computed no diagnostic at all for an
/// inline project, pushed or pulled (measured in the spike, `.zen/evidence/P-015.md`).
pub fn settings(project: &Value) -> Value {
    json!({
        "linkedProjects": [project],
        "cargo": { "buildScripts": { "enable": false } },
        "procMacro": { "enable": false },
        "checkOnSave": false,
        "diagnostics": { "enable": true, "experimental": { "enable": true } }
    })
}

pub fn initialize(id: i64, root: &Path, settings: &Value) -> Value {
    json!({
        "jsonrpc": "2.0", "id": id, "method": "initialize",
        "params": {
            "processId": std::process::id(),
            "rootUri": uri_of(root),
            "capabilities": {
                "textDocument": {
                    "publishDiagnostics": { "relatedInformation": false },
                    "diagnostic": { "dynamicRegistration": false, "relatedDocumentSupport": false },
                    "completion": { "completionItem": { "snippetSupport": false } },
                    "synchronization": { "didSave": false }
                },
                "workspace": { "configuration": true },
                "general": { "positionEncodings": ["utf-16"] },
                "experimental": { "serverStatusNotification": true }
            },
            "initializationOptions": settings
        }
    })
}

pub fn notification(method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "method": method, "params": params })
}

pub fn request(id: i64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// One diagnostic as the editor draws it: zero-based lines and UTF-16 columns.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Diagnostic {
    pub line: u64,
    pub column: u64,
    pub end_line: u64,
    pub end_column: u64,
    /// 1 error, 2 warning, 3 information, 4 hint (the protocol's numbers).
    pub severity: u64,
    pub message: String,
    pub code: String,
}

/// One completion: what it shows and what it writes.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Completion {
    pub label: String,
    pub insert: String,
    pub detail: String,
}

/// What the server said, as the editor needs it.
#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The server answered `initialize`: the client sends `initialized` and opens its files.
    Initialized,
    /// rust-analyzer's status: `quiescent` once it has nothing left to analyse, when the
    /// diagnostics asked for are the whole of them.
    Status { quiescent: bool },
    /// Diagnostics the server pushed for a file.
    Diagnostics { uri: String, items: Vec<Diagnostic> },
    Completions { ticket: i64, items: Vec<Completion> },
    /// The diagnostics of the file asked for by its ticket.
    Pulled { file: i64, items: Vec<Diagnostic> },
    /// A request of ours the server refused or could not answer yet (`code` -32802 "server
    /// cancelled", -32801 "content modified": ask again).
    Refused { pending: Pending, code: i64, message: String },
    /// A request of the server's, with the answer to send back.
    Answer(Value),
    /// A message the editor has no use for.
    Other,
}

/// What a request of ours was for, by its id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    Initialize,
    Completion { ticket: i64 },
    /// The file's diagnostics, asked for (the protocol's pull model): rust-analyzer computes its
    /// own diagnostics for a client that asks and pushed none of them to this one.
    Diagnostics { file: i64 },
}

fn diagnostic(d: &Value) -> Diagnostic {
    let n = |p: &str| d.pointer(p).and_then(Value::as_u64).unwrap_or(0);
    let code = match d.get("code") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    Diagnostic {
        line: n("/range/start/line"),
        column: n("/range/start/character"),
        end_line: n("/range/end/line"),
        end_column: n("/range/end/character"),
        severity: d.get("severity").and_then(Value::as_u64).unwrap_or(1),
        message: d.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
        code,
    }
}

fn completion(c: &Value) -> Completion {
    let s = |p: &str| c.pointer(p).and_then(Value::as_str).map(str::to_string);
    let label = s("/label").unwrap_or_default();
    Completion {
        insert: s("/textEdit/newText").or_else(|| s("/insertText")).unwrap_or_else(|| label.clone()),
        detail: s("/detail").unwrap_or_default(),
        label,
    }
}

/// Read one message of the server's against the requests still waiting for an answer.
pub fn interpret(message: &Value, pending: &mut HashMap<i64, Pending>, settings: &Value) -> Event {
    let id = message.get("id");
    match (message.get("method").and_then(Value::as_str), id) {
        (Some("textDocument/publishDiagnostics"), None) => {
            let uri = message.pointer("/params/uri").and_then(Value::as_str).unwrap_or("").to_string();
            let items = message.pointer("/params/diagnostics").and_then(Value::as_array).map(|a| a.iter().map(diagnostic).collect()).unwrap_or_default();
            Event::Diagnostics { uri, items }
        }
        (Some("experimental/serverStatus"), None) => Event::Status { quiescent: message.pointer("/params/quiescent").and_then(Value::as_bool).unwrap_or(false) },
        // The server asks: its settings are ours, every other request is acknowledged with nothing.
        (Some(method), Some(id)) => {
            let result = if method == "workspace/configuration" {
                let asked = message.pointer("/params/items").and_then(Value::as_array).map_or(1, Vec::len);
                Value::Array(vec![settings.clone(); asked])
            } else {
                Value::Null
            };
            Event::Answer(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
        }
        (None, Some(id)) => match id.as_i64().and_then(|id| pending.remove(&id)) {
            Some(p) if message.get("error").is_some() => Event::Refused {
                pending: p,
                code: message.pointer("/error/code").and_then(Value::as_i64).unwrap_or(0),
                message: message.pointer("/error/message").and_then(Value::as_str).unwrap_or("").to_string(),
            },
            Some(Pending::Initialize) => Event::Initialized,
            Some(Pending::Diagnostics { file }) => {
                let items = message.pointer("/result/items").and_then(Value::as_array).map(|a| a.iter().map(diagnostic).collect()).unwrap_or_default();
                Event::Pulled { file, items }
            }
            Some(Pending::Completion { ticket }) => {
                let result = message.get("result").unwrap_or(&Value::Null);
                let list = result.get("items").unwrap_or(result);
                let items = list.as_array().map(|a| a.iter().map(completion).collect()).unwrap_or_default();
                Event::Completions { ticket, items }
            }
            None => Event::Other,
        },
        _ => Event::Other,
    }
}

#[derive(Debug, Error)]
pub enum ServerError {
    #[error("rust-analyzer does not start: {0}")]
    Start(std::io::Error),
    #[error("rust-analyzer's input is closed: {0}")]
    Write(std::io::Error),
}

/// rust-analyzer, running, its answers read on a thread of their own and handed to `on_message`.
pub struct Server {
    child: Child,
    stdin: ChildStdin,
}

impl Server {
    /// The toolchain's own rust-analyzer: `<sysroot>/bin/rust-analyzer`.
    pub fn binary(sysroot: &Path) -> PathBuf {
        sysroot.join("bin").join(if cfg!(windows) { "rust-analyzer.exe" } else { "rust-analyzer" })
    }

    /// Start `binary` in `cwd`, a folder that is not the project's, and read what it says; its own
    /// log, if `log` names a file, goes there.
    pub fn start(binary: &Path, cwd: &Path, log: Option<&Path>, mut on_message: impl FnMut(Value) + Send + 'static) -> Result<Server, ServerError> {
        let stderr = match log {
            Some(path) => Stdio::from(std::fs::File::create(path).map_err(ServerError::Start)?),
            None => Stdio::null(),
        };
        let mut child = Command::new(binary)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(stderr)
            .spawn()
            .map_err(ServerError::Start)?;
        let stdin = child.stdin.take().ok_or_else(|| ServerError::Start(std::io::Error::other("no input")))?;
        let stdout = child.stdout.take().ok_or_else(|| ServerError::Start(std::io::Error::other("no output")))?;
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut unframer = Unframer::default();
            let mut buf = [0u8; 16 * 1024];
            while let Ok(n) = reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                match unframer.push(&buf[..n]) {
                    Ok(messages) => messages.into_iter().for_each(&mut on_message),
                    Err(_) => break,
                }
            }
        });
        Ok(Server { child, stdin })
    }

    pub fn send(&mut self, message: &Value) -> Result<(), ServerError> {
        self.stdin.write_all(&frame(message)).and_then(|()| self.stdin.flush()).map_err(ServerError::Write)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.send(&request(i64::MAX, "shutdown", Value::Null));
        let _ = self.send(&notification("exit", Value::Null));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
