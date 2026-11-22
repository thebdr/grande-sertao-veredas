//! The spike's point 2: one TOML file as a form and as text, two views of one document; an edit
//! in either shows in the other, a text that does not parse is marked where it fails while the
//! form keeps its state, and saving writes back the file's line endings and its mark.

mod common;

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use common::{Shell, X};

const SAMPLE: &str = "\
# A channel to the sorter.
format = \"veredas-channel\"
version = 1

[link]
role = \"client\"   # dials the device
host = '127.0.0.1'
port = 0x13A4  # 5028
timeout_ms = 2000
keepalive = true
ratio = 1.5

# Framing below.
[[frame]]
name = \"announce\"
fields = [\"id\", \"length\"]  # in order
limits = { min = 4, max = 512 }
\"odd.key\" = \"kept\"
";

const VALUES: [&str; 14] = ["veredas-channel", "1", "client", "127.0.0.1", "5028", "2000", "true", "1.5", "announce", "id", "length", "4", "512", "kept"];

fn project(bytes: &[u8]) -> tempfile::TempDir {
    let dir = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("a scratch folder");
    fs::write(dir.path().join("channel.toml"), bytes).expect("the file");
    dir
}

/// Open the project and the file from the tree, and wait for both views to show it.
fn open(dir: &Path) -> Shell {
    let shell = Shell::start(&[dir.to_str().expect("a UTF-8 path")], &[]);
    shell.click(&shell.row_of("channel.toml"));
    shell.wait_for("oficina/textView", "text", SAMPLE);
    shell.wait_for("oficina/stamp", "text", "veredas-channel 1");
    shell
}

fn fields(shell: &Shell) -> Vec<String> {
    (0..VALUES.len()).map(|i| shell.property(&format!("oficina/field{i}"), "text")).collect()
}

fn wait_fields(shell: &Shell, expected: &[&str]) {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let now = fields(shell);
        if now == expected {
            return;
        }
        assert!(Instant::now() < until, "the form: waited for {expected:?}, last read {now:?}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Select `what` in the text view, at its first place, as the UTF-16 offsets the view counts.
fn select_in_text(shell: &Shell, text: &str, what: &str) {
    let at = text.find(what).unwrap_or_else(|| panic!("no {what:?} in the text"));
    let start = text[..at].encode_utf16().count() as i64;
    let end = start + what.encode_utf16().count() as i64;
    shell.click("oficina/textView");
    shell.wait_for("oficina/textView", "activeFocus", "true");
    shell.invoke("oficina/textView", "select", &[X::Int(start), X::Int(end)]);
    shell.wait_for("oficina/textView", "selectedText", what);
}

#[test]
fn an_edit_in_the_form_shows_in_the_text_and_a_typed_edit_in_the_text_shows_in_the_form() {
    let dir = project(SAMPLE.as_bytes());
    let shell = open(dir.path());
    wait_fields(&shell, &VALUES);
    assert_eq!(shell.property("oficina/field0", "readOnly"), "true", "the stamp is not the form's");
    assert_eq!(shell.property("oficina/field1", "readOnly"), "true");
    assert_eq!(shell.property("oficina/field4", "readOnly"), "false");

    // The form: the port typed in decimal comes back in the text in the old value's radix, its
    // comment kept, every other character of the file unmoved.
    shell.retype("oficina/field4", "5029");
    let after_form = SAMPLE.replacen("port = 0x13A4  # 5028", "port = 0x13A5  # 5028", 1);
    shell.wait_for("oficina/textView", "text", &after_form);
    let mut values = VALUES;
    values[4] = "5029";
    wait_fields(&shell, &values);
    assert_eq!(shell.property("oficina/save", "enabled"), "true", "the file changed");

    // The text: a typed change shows in its field.
    select_in_text(&shell, &after_form, "2000");
    shell.type_text("oficina/textView", "3000");
    values[5] = "3000";
    wait_fields(&shell, &values);
    shell.wait_for("oficina/textView", "text", &after_form.replacen("timeout_ms = 2000", "timeout_ms = 3000", 1));
    assert_eq!(shell.property("oficina/editorError", "text"), "");
    // A value of the wrong type typed into the form is refused by name, the text untouched.
    shell.retype("oficina/field6", "yes");
    shell.wait_for("oficina/editorError", "text", "`link.keepalive`: `yes` is neither `true` nor `false`");
    shell.wait_for("oficina/textView", "text", &after_form.replacen("timeout_ms = 2000", "timeout_ms = 3000", 1));
}

#[test]
fn a_text_that_does_not_parse_is_marked_where_it_fails_and_the_form_keeps_its_state() {
    let dir = project(SAMPLE.as_bytes());
    let shell = open(dir.path());
    wait_fields(&shell, &VALUES);
    select_in_text(&shell, SAMPLE, "2000");
    shell.type_text("oficina/textView", "= =");
    shell.wait_until("oficina/editorError", "text", Duration::from_secs(10), |e| e.starts_with("line 9, column 14: "), "line 9, column 14");
    shell.wait_for("oficina/errorMarker", "visible", "true");
    assert_eq!(shell.property("oficina/errorMarker", "line"), "9");
    assert_eq!(shell.property("oficina/errorMarker", "column"), "14");
    assert_eq!(shell.property("oficina/form", "enabled"), "false");
    assert_eq!(fields(&shell), VALUES, "the form shows the last text that parsed");
    assert_eq!(shell.property("oficina/save", "enabled"), "false");
    // Mended, the text is taken again and the form follows.
    let broken = SAMPLE.replacen("2000", "= =", 1);
    select_in_text(&shell, &broken, "= = =");
    shell.type_text("oficina/textView", "= 2500");
    let mut values = VALUES;
    values[5] = "2500";
    wait_fields(&shell, &values);
    shell.wait_for("oficina/editorError", "text", "");
    shell.wait_for("oficina/form", "enabled", "true");
    shell.wait_for("oficina/errorMarker", "visible", "false");
}

#[test]
fn saving_writes_the_file_whole_with_its_line_endings_and_its_mark() {
    let crlf = format!("\u{feff}{}", SAMPLE.replace('\n', "\r\n"));
    let dir = project(crlf.as_bytes());
    let file = dir.path().join("channel.toml");
    let shell = open(dir.path());
    shell.retype("oficina/field4", "5029");
    shell.wait_for("oficina/save", "enabled", "true");
    shell.click("oficina/save");
    shell.wait_for("oficina/save", "enabled", "false");
    let expected = crlf.replacen("0x13A4", "0x13A5", 1);
    assert_eq!(String::from_utf8(fs::read(&file).expect("read")).expect("UTF-8"), expected);
}
