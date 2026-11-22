//! The spike's point 4: an ORCL file in Monaco, in a page Oficina ships, with rust-analyzer behind
//! Oficina's own language-server client: a planted error marked where it is, completions offered,
//! an edit typed and its mark gone, and nothing of the project's folder run.

mod common;
#[path = "../../model/tests/orcl/mod.rs"]
mod orcl;

use std::time::{Duration, Instant};

use common::{CONTROL, Shell, X};

const PANE: &str = "oficina/codePane";
const MARKERS: &str = "monaco.editor.getModelMarkers({}).map(function (m) { return [m.startLineNumber, m.startColumn, m.endColumn, m.message]; })";
const PLANTED: &str = r#"[[23,36,40,"no method `scna` on type `Ton`"]]"#;
const KEY_SPACE: i64 = 0x20;

/// The planted ORCL project, in a folder whose name does not begin with a dot.
fn project() -> tempfile::TempDir {
    let dir = tempfile::Builder::new().prefix("orcl-").tempdir().expect("a scratch folder");
    orcl::orcl_project(&dir.path().canonicalize().expect("the folder"));
    dir
}

/// Wait until the page's markers read `expected`; how long it took from `since`.
fn wait_markers(shell: &Shell, expected: &str, since: Instant, within: Duration) -> Duration {
    loop {
        let now = shell.eval(PANE, MARKERS);
        if now == expected {
            return since.elapsed();
        }
        assert!(since.elapsed() < within, "the markers: waited {within:?} for {expected}, last read {now}\nthe shell's log:\n{}", shell.log_text());
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Click into the editor, wait for the click to land (it moves the cursor), then put the cursor at
/// a one-based line and column and wait until it is there.
fn focus_at(shell: &Shell, line: u32, column: u32) {
    shell.click("oficina/codeView");
    let until = Instant::now() + Duration::from_secs(10);
    while shell.eval(PANE, "oficinaEditor.hasTextFocus()") != "true" {
        assert!(Instant::now() < until, "the editor never took the focus");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(200));
    shell.eval(PANE, &format!("(oficinaEditor.setPosition({{lineNumber: {line}, column: {column}}}), true)"));
    assert_eq!(shell.eval(PANE, "[oficinaEditor.getPosition().lineNumber, oficinaEditor.getPosition().column]"), format!("[{line},{column}]"));
}

#[test]
fn an_orcl_file_is_marked_where_rust_analyzer_says_and_an_edit_typed_in_monaco_clears_it() {
    let dir = project();
    let shell = Shell::start(&[dir.path().to_str().expect("a UTF-8 path")], &[]);
    shell.click(&shell.row_of("src"));
    let file = shell.row_of("main.rs");
    let opened = Instant::now();
    shell.click(&file);
    // The page is Oficina's own, and it shows the file.
    shell.wait_until(PANE, "probe", Duration::from_secs(1), |_| true, "anything");
    let marked = wait_markers(&shell, PLANTED, opened, Duration::from_secs(10));
    eprintln!("the planted error marked {marked:?} after the file was opened");
    assert_eq!(shell.property("oficina/codeStatus", "text"), "ready");
    assert_eq!(shell.eval(PANE, "location.protocol"), r#""file:""#, "the page is shipped with Oficina, not fetched");

    // Completion, asked as a person asks it (Ctrl+Space), offers the timer's methods.
    focus_at(&shell, 23, 36);
    shell.key("oficina/codeView", KEY_SPACE, CONTROL);
    let offered = |shell: &Shell| shell.eval(PANE, "Array.from(document.querySelectorAll('.suggest-widget .monaco-list-row .label-name')).map(function (e) { return e.textContent; })");
    let until = Instant::now() + Duration::from_secs(10);
    let labels = loop {
        let l = offered(&shell);
        if l.contains(r#""scan""#) || Instant::now() > until {
            break l;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    for method in [r#""scan""#, r#""q""#, r#""et""#] {
        assert!(labels.contains(method), "{method} offered: {labels}");
    }
    // Escape closes the list; it lands when it lands, so the test waits for the list to go.
    shell.key("oficina/codeView", 0x0100_0000, 0);
    let until = Instant::now() + Duration::from_secs(10);
    while offered(&shell) != "[]" {
        assert!(Instant::now() < until, "the list never closed");
        std::thread::sleep(Duration::from_millis(50));
    }

    // An edit typed over the planted error: the mark goes within two seconds.
    shell.eval(PANE, "(oficinaEditor.setSelection(new monaco.Selection(23, 36, 23, 40)), oficinaEditor.focus(), true)");
    assert_eq!(shell.eval(PANE, "oficinaEditor.getModel().getValueInRange(oficinaEditor.getSelection())"), r#""scna""#);
    let typed = Instant::now();
    shell.invoke(PANE, "typeText", &[X::Str("scan".into())]);
    let cleared = wait_markers(&shell, "[]", typed, Duration::from_secs(2));
    eprintln!("the mark gone {cleared:?} after the edit was typed");
    assert_eq!(shell.eval(PANE, "oficinaEditor.getModel().getLineContent(23)"), r#""        outputs.motor = self.timer.scan(scan, inputs.start, ms(500));""#);

    // Nothing of the project's folder ran: no build script, cargo wrapper or toolchain script.
    drop(shell);
    let ran: Vec<String> = std::fs::read_dir(dir.path().join("markers")).expect("markers").map(|e| e.expect("an entry").file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(ran, Vec::<String>::new(), "something of the folder ran");
}
