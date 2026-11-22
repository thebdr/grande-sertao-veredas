//! The spike's point 7: one headless test that drives the interface whole, run by cargo nextest,
//! its exit code the verdict. In one run: a QML control (the form's field) typed into, an edit
//! typed inside Monaco and read back through Oficina's hook, and the workbench's panes laid out
//! anew and given back by a named layout saved and restored with KDDockWidgets' own saver. Panes
//! floated to another monitor are not judged headless (the review's entry 29, its third option).

mod common;
#[path = "../../model/tests/orcl/mod.rs"]
mod orcl;

use std::time::{Duration, Instant};

use common::{Shell, X};

const AREA: &str = "oficina/area";
const CODE: &str = "oficina/codePane";
const CHANNEL: &str = "format = \"veredas-channel\"\nversion = 1\n\n[link]\nhost = '127.0.0.1'\nport = 5028  # the sorter's\n";

/// The layout as the docking area describes it: `[name, open, floating or null, [window, x, y,
/// width, height] or null]` per dock, null where the dock is closed.
fn layout(shell: &Shell) -> String {
    static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let n = CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    shell.invoke(AREA, "describeLayout", &[X::Str(n.to_string())]);
    let prefix = format!("{n} ");
    let got = shell.wait_until(AREA, "layoutState", Duration::from_secs(10), |v| v.starts_with(&prefix), "the layout's description");
    got[prefix.len()..].to_string()
}

/// Read the layout until `ok` holds; the last reading.
fn layout_until(shell: &Shell, what: &str, ok: impl Fn(&str) -> bool) -> String {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let now = layout(shell);
        if ok(&now) {
            return now;
        }
        assert!(Instant::now() < until, "the layout never came to {what}: last read {now}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn one_run_drives_a_form_an_edit_in_monaco_and_a_named_layout_saved_and_restored() {
    let dir = tempfile::Builder::new().prefix("orcl-").tempdir().expect("a scratch folder");
    let root = dir.path().canonicalize().expect("the folder");
    orcl::orcl_project(&root);
    std::fs::write(root.join("channel.toml"), CHANNEL).expect("the channel");
    let shell = Shell::start(&[root.to_str().expect("a UTF-8 path")], &[]);

    // A QML control: the form's field for the port, typed into as a person types; the text follows.
    shell.click(&shell.row_of("channel.toml"));
    shell.wait_for("oficina/textView", "text", CHANNEL);
    shell.retype("oficina/field3", "5029");
    shell.wait_for("oficina/textView", "text", &CHANNEL.replace("5028", "5029"));

    // An edit inside Monaco, typed by the keyboard's path and read back through Oficina's hook.
    shell.click(&shell.row_of("src"));
    shell.click(&shell.row_of("main.rs"));
    let first = orcl::PROGRAM.lines().next().expect("a first line");
    let line1 = "oficinaEditor.getModel() ? oficinaEditor.getModel().getLineContent(1) : null";
    let until = Instant::now() + Duration::from_secs(10);
    while shell.eval(CODE, line1) != serde_json::to_string(first).expect("JSON") {
        assert!(Instant::now() < until, "the editor never showed the file");
        std::thread::sleep(Duration::from_millis(50));
    }
    shell.click("oficina/codeView");
    let until = Instant::now() + Duration::from_secs(10);
    while shell.eval(CODE, "oficinaEditor.hasTextFocus()") != "true" {
        assert!(Instant::now() < until, "the editor never took the focus");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(200));
    shell.eval(CODE, "(oficinaEditor.setPosition({lineNumber: 1, column: 1}), true)");
    shell.invoke(CODE, "typeText", &[X::Str("// typed ".into())]);
    let edited = serde_json::to_string(&format!("// typed {first}")).expect("JSON");
    let until = Instant::now() + Duration::from_secs(10);
    while shell.eval(CODE, line1) != edited {
        assert!(Instant::now() < until, "the typed edit never showed: {}", shell.eval(CODE, line1));
        std::thread::sleep(Duration::from_millis(50));
    }

    // A named layout: saved, the panes laid out anew (a pane closed, another floated out of the
    // window), and restored: every pane back where it was, in the window it was drawn in.
    let saved = layout(&shell);
    let file = root.join("layouts").join("review.json");
    std::fs::create_dir_all(file.parent().expect("a folder")).expect("layouts");
    shell.invoke(AREA, "saveLayout", &[X::Str(file.display().to_string())]);
    shell.wait_for(AREA, "layoutAnswer", "1 saved");
    let written: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).expect("the layout file")).expect("the layout is JSON");
    assert!(written.to_string().contains("\"workbench\""), "the layout names the workbench: {written}");

    shell.invoke(AREA, "closeDock", &[X::Str("table".into())]);
    shell.invoke(AREA, "floatDock", &[X::Str("code".into())]);
    let changed = layout_until(&shell, "the table closed and the code floated", |l| l.contains(r#"["table",false,null,null]"#) && l.contains(r#"["code",true,true,["own","#));
    assert_ne!(changed, saved);

    shell.invoke(AREA, "restoreLayout", &[X::Str(file.display().to_string())]);
    shell.wait_for(AREA, "layoutAnswer", "2 restored");
    let restored = layout_until(&shell, "the saved layout", |l| l == saved);
    eprintln!("the layout saved and restored: {restored}");
    // The page in the pane that floated and came back still holds the edit: it was moved, not
    // reloaded.
    assert_eq!(shell.eval(CODE, line1), edited, "Monaco's text after the pane floated and came back");
    assert_eq!(shell.errors(), Vec::<String>::new());

    drop(shell);
    let ran: Vec<String> = std::fs::read_dir(root.join("markers")).expect("markers").map(|e| e.expect("an entry").file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(ran, Vec::<String>::new(), "something of the folder ran");
}
