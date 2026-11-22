//! The language-server client of point 4, its logic alone and then against the real rust-analyzer:
//! a planted error marked, completions offered, and nothing of the project's folder run.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use oficina_spike_model::lsp::{self, Completion, Diagnostic, Event, FrameError, Pending, Server, Unframer};
use serde_json::{Value, json};

#[test]
fn messages_are_framed_and_read_back_whatever_pieces_they_come_in() {
    let a = json!({"jsonrpc": "2.0", "id": 1, "result": {"ação": "😀"}});
    let b = json!({"jsonrpc": "2.0", "method": "x", "params": [1, 2]});
    let mut bytes = lsp::frame(&a);
    assert!(bytes.starts_with(format!("Content-Length: {}\r\n\r\n", a.to_string().len()).as_bytes()));
    bytes.extend(lsp::frame(&b));
    // Byte by byte, then all at once.
    let mut u = Unframer::default();
    let mut got = Vec::new();
    for byte in &bytes {
        got.extend(u.push(std::slice::from_ref(byte)).expect("frames"));
    }
    assert_eq!(got, vec![a.clone(), b.clone()]);
    assert_eq!(Unframer::default().push(&bytes).expect("frames"), vec![a.clone(), b]);
    // Another header beside the length, the length's name in any case.
    let body = a.to_string();
    let other = format!("content-length: {}\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{body}", body.len());
    assert_eq!(Unframer::default().push(other.as_bytes()).expect("frames"), vec![a]);
    assert_eq!(Unframer::default().push(b"Content-Type: x\r\n\r\n{}").unwrap_err(), FrameError::NoLength("Content-Type: x".into()));
    assert!(matches!(Unframer::default().push(b"Content-Length: 3\r\n\r\nnot"), Err(FrameError::NotJson(_))));
    assert_eq!(lsp::uri_of(Path::new("/a b/ação.rs")), "file:///a%20b/a%C3%A7%C3%A3o.rs");
}

#[test]
fn what_the_server_says_is_read_as_the_editor_needs_it() {
    let settings = json!({"s": 1});
    let mut pending = HashMap::from([(1, Pending::Initialize), (7, Pending::Completion { ticket: 3 }), (8, Pending::Completion { ticket: 4 })]);
    let published = json!({"jsonrpc": "2.0", "method": "textDocument/publishDiagnostics", "params": {"uri": "file:///p/src/main.rs", "diagnostics": [
        {"range": {"start": {"line": 18, "character": 33}, "end": {"line": 18, "character": 37}}, "severity": 1, "code": "E0599", "message": "no method"},
        {"range": {"start": {"line": 2, "character": 0}, "end": {"line": 2, "character": 4}}, "severity": 2, "code": 12, "message": "unused"}
    ]}});
    assert_eq!(
        lsp::interpret(&published, &mut pending, &settings),
        Event::Diagnostics {
            uri: "file:///p/src/main.rs".into(),
            items: vec![
                Diagnostic { line: 18, column: 33, end_line: 18, end_column: 37, severity: 1, message: "no method".into(), code: "E0599".into() },
                Diagnostic { line: 2, column: 0, end_line: 2, end_column: 4, severity: 2, message: "unused".into(), code: "12".into() },
            ]
        }
    );
    // The server's requests: its settings are ours, one per item asked; anything else, nothing.
    let asked = json!({"jsonrpc": "2.0", "id": 9, "method": "workspace/configuration", "params": {"items": [{"section": "rust-analyzer"}, {"section": "x"}]}});
    assert_eq!(lsp::interpret(&asked, &mut pending, &settings), Event::Answer(json!({"jsonrpc": "2.0", "id": 9, "result": [{"s": 1}, {"s": 1}]})));
    let progress = json!({"jsonrpc": "2.0", "id": "p1", "method": "window/workDoneProgress/create", "params": {}});
    assert_eq!(lsp::interpret(&progress, &mut pending, &settings), Event::Answer(json!({"jsonrpc": "2.0", "id": "p1", "result": null})));
    // Answers to ours, each once.
    assert_eq!(lsp::interpret(&json!({"jsonrpc": "2.0", "id": 1, "result": {}}), &mut pending, &settings), Event::Initialized);
    assert_eq!(lsp::interpret(&json!({"jsonrpc": "2.0", "id": 1, "result": {}}), &mut pending, &settings), Event::Other);
    let list = json!({"jsonrpc": "2.0", "id": 7, "result": {"isIncomplete": false, "items": [
        {"label": "scan(…)", "textEdit": {"newText": "scan", "range": {}}, "detail": "fn(&mut self, &Scan, bool, Duration) -> bool"},
        {"label": "q()", "insertText": "q()"},
        {"label": "et"}
    ]}});
    assert_eq!(
        lsp::interpret(&list, &mut pending, &settings),
        Event::Completions {
            ticket: 3,
            items: vec![
                Completion { label: "scan(…)".into(), insert: "scan".into(), detail: "fn(&mut self, &Scan, bool, Duration) -> bool".into() },
                Completion { label: "q()".into(), insert: "q()".into(), detail: String::new() },
                Completion { label: "et".into(), insert: "et".into(), detail: String::new() },
            ]
        }
    );
    pending.insert(10, Pending::Diagnostics { file: 5 });
    pending.insert(11, Pending::Diagnostics { file: 5 });
    let pulled = json!({"jsonrpc": "2.0", "id": 10, "result": {"kind": "full", "items": [
        {"range": {"start": {"line": 22, "character": 35}, "end": {"line": 22, "character": 39}}, "severity": 1, "code": "E0599", "message": "no method `scna` on type `Ton`"}
    ]}});
    assert_eq!(
        lsp::interpret(&pulled, &mut pending, &settings),
        Event::Pulled { file: 5, items: vec![Diagnostic { line: 22, column: 35, end_line: 22, end_column: 39, severity: 1, message: "no method `scna` on type `Ton`".into(), code: "E0599".into() }] }
    );
    let refused = json!({"jsonrpc": "2.0", "id": 11, "error": {"code": -32802, "message": "server cancelled the request", "data": {"retriggerRequest": true}}});
    assert_eq!(lsp::interpret(&refused, &mut pending, &settings), Event::Refused { pending: Pending::Diagnostics { file: 5 }, code: -32802, message: "server cancelled the request".into() });
    let bare = json!({"jsonrpc": "2.0", "id": 8, "result": [{"label": "a"}]});
    assert_eq!(lsp::interpret(&bare, &mut pending, &settings), Event::Completions { ticket: 4, items: vec![Completion { label: "a".into(), insert: "a".into(), detail: String::new() }] });
    assert!(pending.is_empty());
}

/// An ORCL program with an error planted on line 23 (zero-based 22), and the planted things that
/// leave a marker in `markers/` if anything runs them: a build script, a compiler wrapper named by
/// the folder's cargo configuration, and a toolchain file naming a toolchain made of scripts.
pub fn orcl_project(root: &Path) -> PathBuf {
    let markers = root.join("markers");
    std::fs::create_dir_all(root.join("src")).expect("src");
    std::fs::create_dir_all(&markers).expect("markers");
    let program = root.join("src/main.rs");
    std::fs::write(&program, PROGRAM).expect("the program");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"conveyor\"\nversion = \"0.1.0\"\nedition = \"2024\"\nbuild = \"build.rs\"\n").expect("a manifest");
    std::fs::write(root.join("build.rs"), format!("fn main() {{ std::fs::write({:?}, \"\").unwrap(); }}\n", markers.join("build-script"))).expect("a build script");
    let script = |name: &str| {
        let path = root.join("tools").join(name);
        std::fs::create_dir_all(path.parent().expect("a folder")).expect("tools");
        std::fs::write(&path, format!("#!/bin/sh\ntouch {:?}\nexit 1\n", markers.join(name.replace('/', "-")))).expect("a script");
        #[cfg(unix)]
        std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o755)).expect("executable");
        path
    };
    let wrapper = script("wrapper");
    std::fs::create_dir_all(root.join(".cargo")).expect(".cargo");
    std::fs::write(root.join(".cargo/config.toml"), format!("[build]\nrustc-wrapper = {:?}\n", wrapper)).expect("a cargo configuration");
    for tool in ["toolchain/bin/rustc", "toolchain/bin/cargo", "toolchain/bin/rust-analyzer"] {
        script(tool);
    }
    std::fs::write(root.join("rust-toolchain.toml"), format!("[toolchain]\npath = {:?}\n", root.join("tools/toolchain"))).expect("a toolchain file");
    program
}

pub const PROGRAM: &str = "\
//! A conveyor that runs once its start button has been held for half a second.
use orcl_std::{Program, Scan, Ton, ms};

#[derive(Clone, Copy, Default)]
pub struct Inputs {
    pub start: bool,
}

#[derive(Clone, Copy, Default)]
pub struct Outputs {
    pub motor: bool,
}

#[derive(Default)]
pub struct Conveyor {
    timer: Ton,
}

impl Program for Conveyor {
    type Inputs = Inputs;
    type Outputs = Outputs;
    fn scan(&mut self, scan: &Scan, inputs: &Inputs, outputs: &mut Outputs) {
        outputs.motor = self.timer.scna(scan, inputs.start, ms(500));
    }
}

fn main() {}
";

pub fn sysroot() -> PathBuf {
    let out = std::process::Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into())).args(["--print", "sysroot"]).output().expect("rustc runs");
    PathBuf::from(String::from_utf8(out.stdout).expect("UTF-8").trim())
}

pub fn orcl_std() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../crates/orcl-std/src/lib.rs").canonicalize().expect("orcl-std in this repository")
}

#[test]
fn rust_analyzer_marks_the_planted_error_offers_the_timer_s_methods_and_runs_nothing_of_the_folder() {
    // Not a hidden folder: rust-analyzer loads no source under a folder whose name begins with a dot.
    let dir = tempfile::Builder::new().prefix("orcl-").tempdir().expect("a scratch folder");
    let root = dir.path().canonicalize().expect("the folder");
    let program = orcl_project(&root);
    let away = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("a folder of Oficina's");
    let sysroot = sysroot();
    let settings = lsp::settings(&lsp::orcl_project(&program, &orcl_std(), &sysroot));
    let (tx, rx) = mpsc::channel::<Value>();
    let started = Instant::now();
    let log = std::env::var_os("LSP_TRACE").map(|_| Path::new(env!("CARGO_TARGET_TMPDIR")).join("rust-analyzer.log"));
    let mut server = Server::start(&Server::binary(&sysroot), away.path(), log.as_deref(), move |m| {
        let _ = tx.send(m);
    })
    .expect("rust-analyzer starts");
    let mut pending = HashMap::from([(1, Pending::Initialize)]);
    server.send(&lsp::initialize(1, &root, &settings)).expect("sent");
    let uri = lsp::uri_of(&program);
    let mut marked = None;
    let mut completions = None;
    let mut ask_again: Option<Instant> = None;
    let mut next = 2;
    let until = Instant::now() + Duration::from_secs(if std::env::var_os("LSP_TRACE").is_some() { 40 } else { 120 });
    while (marked.is_none() || completions.is_none()) && Instant::now() < until {
        // Ask for the file's diagnostics until they hold the planted error (the pull model).
        if ask_again.is_some_and(|at| Instant::now() >= at) && marked.is_none() {
            ask_again = None;
            pending.insert(next, Pending::Diagnostics { file: 1 });
            server.send(&lsp::request(next, "textDocument/diagnostic", json!({"textDocument": {"uri": uri}}))).expect("sent");
            next += 1;
        }
        let Ok(message) = rx.recv_timeout(Duration::from_millis(50)) else { continue };
        if std::env::var_os("LSP_TRACE").is_some() {
            let text = message.to_string();
            eprintln!("{:?} <- {}", started.elapsed(), &text[..text.len().min(400)]);
        }
        match lsp::interpret(&message, &mut pending, &settings) {
            Event::Initialized => {
                server.send(&lsp::notification("initialized", json!({}))).expect("sent");
                server.send(&lsp::notification("textDocument/didOpen", json!({"textDocument": {"uri": uri, "languageId": "rust", "version": 1, "text": PROGRAM}}))).expect("sent");
                ask_again = Some(Instant::now());
            }
            Event::Pulled { items, .. } => match items.into_iter().find(|d| d.message.contains("scna")) {
                Some(d) => {
                    marked = Some((d, started.elapsed()));
                    // Completion after `self.timer.` on the planted line, as the editor asks it.
                    pending.insert(next, Pending::Completion { ticket: 1 });
                    server.send(&lsp::request(next, "textDocument/completion", json!({"textDocument": {"uri": uri}, "position": {"line": 22, "character": 35}}))).expect("sent");
                    next += 1;
                }
                None => ask_again = Some(Instant::now() + Duration::from_millis(250)),
            },
            Event::Refused { pending: Pending::Diagnostics { .. }, .. } => ask_again = Some(Instant::now() + Duration::from_millis(250)),
            Event::Completions { items, .. } => completions = Some(items),
            Event::Answer(a) => server.send(&a).expect("sent"),
            _ => {}
        }
    }
    let (d, took) = marked.expect("the planted error is marked");
    eprintln!("marked in {took:?}: {d:?}");
    assert_eq!((d.line, d.column, d.end_line, d.end_column, d.severity), (22, 35, 22, 39, 1), "{d:?}");
    let labels: Vec<String> = completions.expect("completions come").into_iter().map(|c| c.label).collect();
    eprintln!("completions: {labels:?}");
    for method in ["scan", "q", "et"] {
        assert!(labels.iter().any(|l| l == method || l.starts_with(&format!("{method}("))), "{method} in {labels:?}");
    }
    drop(server);
    let ran: Vec<String> = std::fs::read_dir(root.join("markers")).expect("markers").map(|e| e.expect("an entry").file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(ran, Vec::<String>::new(), "something of the folder ran");
}
