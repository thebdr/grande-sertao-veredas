//! The code editor's other half (the spike's point 4): the object the Monaco page reaches through
//! a web channel, driving the model's language-server client. The page sends what is typed and asks
//! for completions; this object answers with the file to show, the diagnostics to mark and the
//! completions to offer. rust-analyzer's messages are read on a thread of the client's and handed
//! to Qt's thread here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::QString;
use oficina_spike_model::lsp::{self, Event, Pending, Server};
use serde_json::{Value, json};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    unsafe extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(QString, status)]
        #[qproperty(QString, page_url, cxx_name = "pageUrl")]
        #[qproperty(QString, path)]
        type LanguageClient = super::LanguageClientRust;

        /// The file to show: its address and its text.
        #[qsignal]
        fn opened(self: Pin<&mut LanguageClient>, uri: QString, text: QString);
        /// The file's diagnostics, as JSON: `[{line, column, end_line, end_column, severity, message, code}]`.
        #[qsignal]
        fn diagnostics(self: Pin<&mut LanguageClient>, json: QString);
        /// The completions asked for by `ticket`, as JSON: `[{label, insert, detail}]`.
        #[qsignal]
        fn completions(self: Pin<&mut LanguageClient>, ticket: i32, json: QString);

        /// Open an ORCL file: rust-analyzer started on its project, the file shown.
        #[qinvokable]
        fn open(self: Pin<&mut LanguageClient>, path: &QString) -> bool;
        /// The page is ready to show a file.
        #[qinvokable]
        #[cxx_name = "pageReady"]
        fn page_ready(self: Pin<&mut LanguageClient>);
        /// The page's text changed.
        #[qinvokable]
        fn edit(self: Pin<&mut LanguageClient>, text: &QString);
        /// Completions at a zero-based line and UTF-16 column, answered by `completions(ticket, ..)`.
        #[qinvokable]
        fn complete(self: Pin<&mut LanguageClient>, ticket: i32, line: i32, column: i32);
    }

    impl cxx_qt::Threading for LanguageClient {}
}

/// Where the editor's page is: shipped with Oficina, never fetched.
const PAGE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/web/editor.html");

pub struct LanguageClientRust {
    server: Option<Server>,
    pending: HashMap<i64, Pending>,
    next: i64,
    settings: Value,
    uri: String,
    text: String,
    version: i64,
    ready: bool,
    status: QString,
    page_url: QString,
    path: QString,
}

impl Default for LanguageClientRust {
    fn default() -> Self {
        LanguageClientRust {
            server: None,
            pending: HashMap::new(),
            next: 1,
            settings: Value::Null,
            uri: String::new(),
            text: String::new(),
            version: 0,
            ready: false,
            status: QString::from("idle"),
            page_url: QString::from(format!("file://{PAGE}").as_str()),
            path: QString::default(),
        }
    }
}

/// The toolchain's sysroot, asked of `rustc` in a folder of Oficina's, never the project's (where a
/// toolchain file would choose another).
fn sysroot() -> Option<PathBuf> {
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let out = std::process::Command::new(rustc).args(["--print", "sysroot"]).current_dir(std::env::temp_dir()).output().ok()?;
    out.status.success().then(|| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

/// `orcl-std` as this repository holds it; Oficina would ship it.
fn orcl_std() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../crates/orcl-std/src/lib.rs")
}

/// An ORCL project's folder: the folder holding `src/`, or the file's own.
fn project_of(file: &Path) -> PathBuf {
    let parent = file.parent().unwrap_or(Path::new("/"));
    match parent.file_name() {
        Some(n) if n == "src" => parent.parent().unwrap_or(parent).to_path_buf(),
        _ => parent.to_path_buf(),
    }
}

impl qobject::LanguageClient {
    fn send(mut self: Pin<&mut Self>, message: &Value) {
        let failed = match self.as_mut().rust_mut().server.as_mut() {
            Some(s) => s.send(message).err(),
            None => None,
        };
        if let Some(e) = failed {
            self.as_mut().set_status(QString::from(e.to_string().as_str()));
        }
    }

    fn ask(mut self: Pin<&mut Self>, method: &str, params: Value, pending: Pending) {
        let id = self.next;
        {
            let mut r = self.as_mut().rust_mut();
            r.next += 1;
            r.pending.insert(id, pending);
        }
        self.send(&lsp::request(id, method, params));
    }

    fn pull(self: Pin<&mut Self>) {
        let uri = self.uri.clone();
        self.ask("textDocument/diagnostic", json!({"textDocument": {"uri": uri}}), Pending::Diagnostics { file: 1 });
    }

    fn show(mut self: Pin<&mut Self>) {
        if self.ready && !self.uri.is_empty() {
            let (uri, text) = (QString::from(self.uri.as_str()), QString::from(self.text.as_str()));
            self.as_mut().opened(uri, text);
        }
    }

    fn open(mut self: Pin<&mut Self>, path: &QString) -> bool {
        let file = PathBuf::from(path.to_string());
        let text = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) => {
                self.as_mut().set_status(QString::from(e.to_string().as_str()));
                return false;
            }
        };
        let Some(sysroot) = sysroot() else {
            self.as_mut().set_status(QString::from("no Rust toolchain: rustc --print sysroot failed"));
            return false;
        };
        let settings = lsp::settings(&lsp::orcl_project(&file, &orcl_std(), &sysroot));
        let thread = self.qt_thread();
        let started = Server::start(&Server::binary(&sysroot), &std::env::temp_dir(), None, move |message| {
            let _ = thread.queue(move |q: Pin<&mut qobject::LanguageClient>| q.receive(message));
        });
        let server = match started {
            Ok(s) => s,
            Err(e) => {
                self.as_mut().set_status(QString::from(e.to_string().as_str()));
                return false;
            }
        };
        {
            let mut r = self.as_mut().rust_mut();
            // The old server, if any, ends here.
            r.server = Some(server);
            r.pending.clear();
            r.uri = lsp::uri_of(&file);
            r.text = text;
            r.version = 1;
            r.settings = settings.clone();
        }
        self.as_mut().set_path(path.clone());
        self.as_mut().set_status(QString::from("starting"));
        self.as_mut().ask_initialize(&project_of(&file), &settings);
        self.show();
        true
    }

    fn ask_initialize(mut self: Pin<&mut Self>, root: &Path, settings: &Value) {
        let id = self.next;
        {
            let mut r = self.as_mut().rust_mut();
            r.next += 1;
            r.pending.insert(id, Pending::Initialize);
        }
        self.send(&lsp::initialize(id, root, settings));
    }

    fn page_ready(mut self: Pin<&mut Self>) {
        self.as_mut().rust_mut().ready = true;
        self.show();
    }

    fn edit(mut self: Pin<&mut Self>, text: &QString) {
        let text = text.to_string();
        if text == self.text || self.server.is_none() {
            return;
        }
        let (uri, version) = {
            let mut r = self.as_mut().rust_mut();
            r.version += 1;
            r.text = text.clone();
            (r.uri.clone(), r.version)
        };
        self.as_mut().send(&lsp::notification("textDocument/didChange", json!({"textDocument": {"uri": uri, "version": version}, "contentChanges": [{"text": text}]})));
        self.pull();
    }

    fn complete(self: Pin<&mut Self>, ticket: i32, line: i32, column: i32) {
        let uri = self.uri.clone();
        self.ask("textDocument/completion", json!({"textDocument": {"uri": uri}, "position": {"line": line, "character": column}}), Pending::Completion { ticket: i64::from(ticket) });
    }

    /// One message of rust-analyzer's, on Qt's thread.
    fn receive(mut self: Pin<&mut Self>, message: Value) {
        let settings = self.settings.clone();
        let event = lsp::interpret(&message, &mut self.as_mut().rust_mut().pending, &settings);
        match event {
            Event::Initialized => {
                let (uri, text) = (self.uri.clone(), self.text.clone());
                self.as_mut().send(&lsp::notification("initialized", json!({})));
                self.as_mut().send(&lsp::notification("textDocument/didOpen", json!({"textDocument": {"uri": uri, "languageId": "rust", "version": 1, "text": text}})));
                self.as_mut().set_status(QString::from("indexing"));
            }
            Event::Status { quiescent: true } => {
                self.as_mut().set_status(QString::from("ready"));
                self.pull();
            }
            Event::Pulled { items, .. } => {
                let json = serde_json::to_string(&items).unwrap_or_else(|_| "[]".into());
                self.as_mut().diagnostics(QString::from(json.as_str()));
            }
            Event::Diagnostics { uri, items } if uri == self.uri => {
                let json = serde_json::to_string(&items).unwrap_or_else(|_| "[]".into());
                self.as_mut().diagnostics(QString::from(json.as_str()));
            }
            Event::Refused { pending: Pending::Diagnostics { .. }, .. } => {
                // Not ready for this text yet: ask again shortly.
                let thread = self.qt_thread();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(100));
                    let _ = thread.queue(|q: Pin<&mut qobject::LanguageClient>| q.pull());
                });
            }
            Event::Completions { ticket, items } => {
                let json = serde_json::to_string(&items).unwrap_or_else(|_| "[]".into());
                self.as_mut().completions(ticket as i32, QString::from(json.as_str()));
            }
            Event::Answer(a) => self.send(&a),
            _ => {}
        }
    }
}
