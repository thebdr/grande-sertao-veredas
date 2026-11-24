//! The spike's point 1: a project folder opened and its tree shown, read lazily, a re-read that
//! keeps the open folders and the selection, every name drawn as its characters.

mod common;

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use common::Shell;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().expect("a parent")).expect("the folder");
    fs::write(p, text).expect("the file");
}

fn stamped(format: &str, version: u32) -> String {
    format!("format = \"{format}\"\nversion = {version}\n")
}

/// A folder for a test's project: under `OFICINA_SCRATCH` when a run names one (a guest names its
/// own disk, its build folder being mounted from elsewhere), under the build's own else.
fn scratch() -> tempfile::TempDir {
    let base = std::env::var_os("OFICINA_SCRATCH").map(std::path::PathBuf::from).unwrap_or_else(|| env!("CARGO_TARGET_TMPDIR").into());
    tempfile::tempdir_in(base).expect("a scratch folder")
}

/// A file name that is markup: a tag where the system allows one, entities on Windows, whose names
/// cannot hold `<` or `>`. Drawn as rich text, either would show other characters than the name.
const MARKUP: &str = if cfg!(windows) { "&lt;b&gt;RUN.txt" } else { "<b>RUN.txt" };

/// The planted project of the model's own tests, and a file whose name is markup.
fn project() -> tempfile::TempDir {
    let dir = scratch();
    let r = dir.path();
    write(r, "project.toml", &format!("{}name = \"the press hall\"\n", stamped("veredas-project", 1)));
    write(r, "channel.toml", &stamped("veredas-channel", 1));
    write(r, "ação.toml", &stamped("veredas-bench", 1));
    write(r, "later.toml", &stamped("veredas-panel", 3));
    write(r, "B.toml", "x = 1\n");
    write(r, "broken.toml", "x = [\n");
    write(r, "Cargo.toml", "[package]\nname = \"orcl-demo\"\n");
    write(r, "big.toml", &"#".repeat((1 << 20) + 1));
    write(r, "layout.svg", "<svg/>");
    write(r, "notes.txt", "notes");
    write(r, ".cargo/config.toml", "[build]\nrustc-wrapper = \"./wrapper.sh\"\n");
    write(r, "alpha/deep.toml", &stamped("veredas-binding", 1));
    write(r, &format!("alpha/{MARKUP}"), "a name that is markup");
    fs::create_dir_all(r.join("alpha/inner")).expect("a folder");
    fs::create_dir_all(r.join("Zeta")).expect("a folder");
    dir
}

const ROOT: [&str; 13] = [
    ".cargo",
    "alpha",
    "Zeta",
    "ação.toml | veredas-bench 1 · Maquinar",
    "B.toml | no `format`: a TOML file states `format` and `version`",
    "big.toml | too large to read: 1048577 bytes",
    "broken.toml | BROKEN",
    "Cargo.toml | cargo manifest",
    "channel.toml | veredas-channel 1 · Core",
    "later.toml | veredas-panel 3",
    "layout.svg",
    "notes.txt",
    "project.toml | veredas-project 1 · Portal",
];

/// Core's refusal of `broken.toml` runs over several lines; the rows compare its first words.
fn rows_brief(shell: &Shell) -> Vec<String> {
    shell.rows().into_iter().map(|r| if r.starts_with("broken.toml | not a TOML file: ") { "broken.toml | BROKEN".to_string() } else { r }).collect()
}

fn wait_rows(shell: &Shell, expected: &[&str]) {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let rows = rows_brief(shell);
        if rows == expected {
            return;
        }
        assert!(Instant::now() < until, "waited for the rows {expected:#?}\nlast read {rows:#?}\nthe shell's log:\n{}", shell.log_text());
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn row_named(shell: &Shell, name: &str) -> String {
    let i = rows_brief(shell).iter().position(|r| r == name || r.starts_with(&format!("{name} | "))).unwrap_or_else(|| panic!("no row {name}"));
    format!("oficina/row{i}")
}

#[test]
fn the_project_folder_is_drawn_row_for_row_folders_first_each_toml_file_named_by_its_stamp() {
    let dir = project();
    let shell = Shell::start(&[dir.path().to_str().expect("a UTF-8 path")], &[]);
    wait_rows(&shell, &ROOT);
    assert_eq!(shell.property("oficina/projectError", "text"), "");
}

#[test]
fn a_folder_is_read_when_it_opens_and_again_only_when_refreshed() {
    let dir = project();
    let r = dir.path();
    let shell = Shell::start(&[r.to_str().expect("a UTF-8 path")], &[]);
    wait_rows(&shell, &ROOT);
    // Written after the root was read and before alpha opens: alpha is read when it opens.
    write(r, "alpha/late.toml", &stamped("veredas-catalog", 1));
    shell.click(&row_named(&shell, "alpha"));
    let mut opened: Vec<&str> = ROOT.to_vec();
    opened.splice(2..2, ["inner", MARKUP, "deep.toml | veredas-binding 1 · Supervisão", "late.toml | veredas-catalog 1 · Core"]);
    wait_rows(&shell, &opened);
    // A name that is markup is drawn as its characters.
    let markup = row_named(&shell, MARKUP);
    assert_eq!(shell.property(&format!("{markup}/name"), "text"), MARKUP);
    assert_eq!(shell.property(&format!("{markup}/name"), "textFormat"), "PlainText");
    // Written after alpha was read: not drawn until the tree reads it again.
    write(r, "alpha/zz.toml", &stamped("veredas-catalog", 1));
    std::thread::sleep(Duration::from_millis(300));
    wait_rows(&shell, &opened);
    shell.click("oficina/refreshProject");
    opened.insert(6, "zz.toml | veredas-catalog 1 · Core");
    wait_rows(&shell, &opened);
}

#[test]
fn a_refresh_keeps_the_open_folders_and_the_selection() {
    let dir = project();
    let r = dir.path();
    let shell = Shell::start(&[r.to_str().expect("a UTF-8 path")], &[]);
    wait_rows(&shell, &ROOT);
    shell.click(&row_named(&shell, "alpha"));
    shell.wait_for("oficina/row2", "label", "inner");
    let project = row_named(&shell, "project.toml");
    shell.click(&project);
    shell.wait_for(&project, "current", "true");
    // A file comes before the selection, one leaves before it, one gains a stamp.
    write(r, "new.toml", &stamped("veredas-catalog", 1));
    fs::remove_file(r.join("notes.txt")).expect("removed");
    write(r, "B.toml", &stamped("veredas-catalog", 1));
    shell.click("oficina/refreshProject");
    let expected = [
        ".cargo",
        "alpha",
        "inner",
        MARKUP,
        "deep.toml | veredas-binding 1 · Supervisão",
        "Zeta",
        "ação.toml | veredas-bench 1 · Maquinar",
        "B.toml | veredas-catalog 1 · Core",
        "big.toml | too large to read: 1048577 bytes",
        "broken.toml | BROKEN",
        "Cargo.toml | cargo manifest",
        "channel.toml | veredas-channel 1 · Core",
        "later.toml | veredas-panel 3",
        "layout.svg",
        "new.toml | veredas-catalog 1 · Core",
        "project.toml | veredas-project 1 · Portal",
    ];
    wait_rows(&shell, &expected);
    // alpha is still open (its rows are drawn) and project.toml still the current row.
    let project = row_named(&shell, "project.toml");
    shell.wait_for(&project, "current", "true");
    let others: Vec<String> = (0..expected.len()).map(|i| format!("oficina/row{i}")).filter(|p| *p != project).map(|p| shell.property(&p, "current")).collect();
    assert!(others.iter().all(|c| c == "false"), "one current row: {others:?}");
}

/// A timing of the product: judged on a release build (`cargo test --release`), alone on the
/// machine; a debug build is several times slower in the model alone (evidence, P-015).
#[test]
#[cfg_attr(debug_assertions, ignore = "a timing: run it with --release")]
fn a_folder_of_100000_files_opens_in_under_a_second() {
    let dir = scratch();
    let many = dir.path().join("many");
    fs::create_dir_all(&many).expect("a folder");
    for i in 0..100_000 {
        fs::File::create(many.join(format!("f{i:06}.log"))).expect("a file");
    }
    let shell = Shell::start(&[dir.path().to_str().expect("a UTF-8 path")], &[]);
    shell.wait_for("oficina/row0", "label", "many");
    let start = Instant::now();
    shell.click("oficina/row0");
    shell.wait_for("oficina/row1", "label", "f000000.log");
    let took = start.elapsed();
    eprintln!("a folder of 100000 files opened in {took:?}");
    assert!(took < Duration::from_secs(1), "took {took:?}");
}
