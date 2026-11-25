//! The project tree (C-251), driven through the window as a person drives it: a project folder
//! opened and its tree shown, read lazily, a re-read that keeps the open folders and the selection,
//! every name drawn as its characters.

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

/// Core's own words for a text it refuses (C-045): what the row must show after the name.
fn refusal(text: &str) -> String {
    veredas_core::schema::stamp(text, "a TOML file").expect_err("core refuses it").to_string()
}

/// The planted project's first level, as the tree draws it.
fn root_rows() -> Vec<String> {
    vec![
        ".cargo".into(),
        "alpha".into(),
        "Zeta".into(),
        "ação.toml | veredas-bench 1 · Maquinar".into(),
        format!("B.toml | {}", refusal("x = 1\n")),
        "big.toml | too large to read: 1048577 bytes".into(),
        format!("broken.toml | {}", refusal("x = [\n")),
        "Cargo.toml | cargo manifest".into(),
        "channel.toml | veredas-channel 1 · Core".into(),
        "later.toml | veredas-panel 3".into(),
        "layout.svg".into(),
        "notes.txt".into(),
        "project.toml | veredas-project 1 · Portal".into(),
    ]
}

fn wait_rows(shell: &Shell, expected: &[String]) {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let rows = shell.rows();
        if rows == expected {
            return;
        }
        assert!(Instant::now() < until, "waited for the rows {expected:#?}\nlast read {rows:#?}\nthe shell's log:\n{}", shell.log_text());
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn row_named(shell: &Shell, name: &str) -> String {
    let i = shell.rows().iter().position(|r| r == name || r.starts_with(&format!("{name} | "))).unwrap_or_else(|| panic!("no row {name}"));
    format!("oficina/row{i}")
}

/// How many folders the model has read from the disk since the project opened.
fn reads(shell: &Shell) -> u64 {
    shell.property("oficina/projectPane", "reads").parse().expect("a count")
}

fn rows_with(base: &[String], at: usize, inserted: &[&str]) -> Vec<String> {
    let mut v = base.to_vec();
    v.splice(at..at, inserted.iter().map(|s| s.to_string()));
    v
}

#[test]
fn the_project_folder_is_drawn_row_for_row_folders_first_each_toml_file_named_by_its_stamp() {
    let dir = project();
    let shell = Shell::start(&[dir.path().to_str().expect("a UTF-8 path")], &[]);
    wait_rows(&shell, &root_rows());
    assert_eq!(shell.property("oficina/projectError", "text"), "");
    assert_eq!(reads(&shell), 1, "the root read, and nothing else");
}

/// Beyond the planted project: names ordered with case folded past ASCII (an accented capital
/// beside its lower case), the folder named after an option, a stamped file of exactly the
/// reader's limit read, and a release file named by the tool that reads it.
#[test]
fn names_fold_their_case_past_ascii_an_option_comes_first_and_the_limit_is_read() {
    let dir = scratch();
    let r = dir.path();
    for name in ["Ágape.txt", "ábaco.txt", "b.txt", "A.txt"] {
        write(r, name, "text");
    }
    let stamp = stamped("veredas-bench", 1);
    write(r, "exact.toml", &format!("{stamp}{}", "#".repeat((1 << 20) - stamp.len())));
    assert_eq!(fs::metadata(r.join("exact.toml")).expect("planted").len(), 1 << 20);
    write(r, "release.toml", &stamped("veredas-release", 1));
    fs::create_dir_all(r.join("Zeta")).expect("a folder");
    fs::create_dir_all(r.join("alpha")).expect("a folder");
    let shell = Shell::start(&["--an-option", r.to_str().expect("a UTF-8 path")], &[]);
    let expected: Vec<String> = ["alpha", "Zeta", "A.txt", "b.txt", "exact.toml | veredas-bench 1 · Maquinar", "release.toml | veredas-release 1 · Alforge", "ábaco.txt", "Ágape.txt"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    wait_rows(&shell, &expected);
    assert_eq!(shell.property("oficina/projectError", "text"), "");
}

#[test]
fn a_folder_is_read_when_it_opens_and_again_only_when_refreshed() {
    let dir = project();
    let r = dir.path();
    let shell = Shell::start(&[r.to_str().expect("a UTF-8 path")], &[]);
    let root = root_rows();
    wait_rows(&shell, &root);
    // Written after the root was read and before alpha opens: alpha is read when it opens, once.
    write(r, "alpha/late.toml", &stamped("veredas-catalog", 1));
    assert_eq!(reads(&shell), 1, "alpha not read before it opens");
    shell.click(&row_named(&shell, "alpha"));
    let opened = rows_with(&root, 2, &["inner", MARKUP, "deep.toml | veredas-binding 1 · Supervisão", "late.toml | veredas-catalog 1 · Core"]);
    wait_rows(&shell, &opened);
    assert_eq!(reads(&shell), 2, "alpha read when it opened, once");
    // A name that is markup is drawn as its characters.
    let markup = row_named(&shell, MARKUP);
    assert_eq!(shell.property(&format!("{markup}/name"), "text"), MARKUP);
    assert_eq!(shell.property(&format!("{markup}/name"), "textFormat"), "PlainText");
    // Written after alpha was read: not drawn, and nothing read, until Refresh, the project pane's
    // dock closed and opened again meanwhile.
    write(r, "alpha/zz.toml", &stamped("veredas-catalog", 1));
    std::thread::sleep(Duration::from_millis(300));
    shell.invoke("oficina/area", "closeDock", &[common::X::Str("project".into())]);
    shell.invoke("oficina/area", "openDock", &[common::X::Str("project".into())]);
    let until = Instant::now() + Duration::from_secs(10);
    while !shell.visible("oficina/projectPane") {
        assert!(Instant::now() < until, "the pane not shown again");
        std::thread::sleep(Duration::from_millis(50));
    }
    wait_rows(&shell, &opened);
    assert_eq!(reads(&shell), 2, "nothing read again before Refresh");
    shell.click("oficina/refreshProject");
    let refreshed = rows_with(&opened, 6, &["zz.toml | veredas-catalog 1 · Core"]);
    wait_rows(&shell, &refreshed);
    assert_eq!(reads(&shell), 4, "Refresh read the root and alpha again");
    // alpha closed: Refresh still reads it (what the model holds of it stays current for when it
    // opens again), and leaves it closed.
    shell.click(&row_named(&shell, "alpha"));
    let closed = root.clone();
    wait_rows(&shell, &closed);
    shell.click("oficina/refreshProject");
    std::thread::sleep(Duration::from_millis(300));
    wait_rows(&shell, &closed);
    assert_eq!(reads(&shell), 6, "Refresh read every folder read so far, alpha closed among them");
}

#[test]
fn a_refresh_keeps_the_open_folders_and_the_selection() {
    let dir = project();
    let r = dir.path();
    let shell = Shell::start(&[r.to_str().expect("a UTF-8 path")], &[]);
    wait_rows(&shell, &root_rows());
    // alpha open, and inner open inside it (empty when it opens).
    shell.click(&row_named(&shell, "alpha"));
    shell.wait_for("oficina/row2", "label", "inner");
    shell.click("oficina/row2");
    let until = Instant::now() + Duration::from_secs(10);
    while reads(&shell) != 3 {
        assert!(Instant::now() < until, "inner never read: {} reads", reads(&shell));
        std::thread::sleep(Duration::from_millis(50));
    }
    let project = row_named(&shell, "project.toml");
    shell.click(&project);
    shell.wait_for(&project, "current", "true");
    // Before the current row: a folder comes before alpha, a file comes and one leaves, one gains a
    // stamp, so the current row moves; and a file comes inside inner, two levels down.
    fs::create_dir_all(r.join("aaa")).expect("a folder");
    write(r, "new.toml", &stamped("veredas-catalog", 1));
    write(r, "new2.toml", &stamped("veredas-catalog", 1));
    fs::remove_file(r.join("notes.txt")).expect("removed");
    write(r, "B.toml", &stamped("veredas-catalog", 1));
    write(r, "alpha/inner/b.txt", "text");
    shell.click("oficina/refreshProject");
    let expected: Vec<String> = [
        ".cargo",
        "aaa",
        "alpha",
        "inner",
        "b.txt",
        MARKUP,
        "deep.toml | veredas-binding 1 · Supervisão",
        "Zeta",
        "ação.toml | veredas-bench 1 · Maquinar",
        "B.toml | veredas-catalog 1 · Core",
        "big.toml | too large to read: 1048577 bytes",
    ]
    .iter()
    .map(|s| s.to_string())
    .chain([format!("broken.toml | {}", refusal("x = [\n"))])
    .chain(
        ["Cargo.toml | cargo manifest", "channel.toml | veredas-channel 1 · Core", "later.toml | veredas-panel 3", "layout.svg", "new.toml | veredas-catalog 1 · Core", "new2.toml | veredas-catalog 1 · Core", "project.toml | veredas-project 1 · Portal"]
            .iter()
            .map(|s| s.to_string()),
    )
    .collect();
    wait_rows(&shell, &expected);
    // The same folders open (alpha's and inner's rows drawn under them, by name) and the same file
    // current, by name, though every row before it moved.
    let project = row_named(&shell, "project.toml");
    assert_eq!(project, "oficina/row18", "project.toml moved down three rows");
    shell.wait_for(&project, "current", "true");
    let others: Vec<String> = (0..expected.len()).map(|i| format!("oficina/row{i}")).filter(|p| *p != project).map(|p| shell.property(&p, "current")).collect();
    assert!(others.iter().all(|c| c == "false"), "one current row: {others:?}");
}

/// Nothing reads the disk again but the Refresh button: in the window's QML the tree is opened
/// once, from the command line's argument, and refreshed once, from the button, the one other
/// `open(` being a dock's, and no timer; in the bridge a folder is read only where the view
/// fetches it and where Refresh reads it, a project opened in one place, and Refresh's two
/// functions never called but as the button calls them. A reading, against accidents (a timer, a
/// watcher or a reload on its own would show here; the counted reads above show one only within
/// each test's span).
#[test]
fn only_the_refresh_button_reads_again() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut calls = Vec::new();
    for qml in ["qml/Main.qml", "qml/ProjectPane.qml"] {
        let text = fs::read_to_string(dir.join(qml)).expect("the QML");
        for line in text.lines().map(str::trim).filter(|l| !l.starts_with("//")) {
            let opens = line.match_indices("open(").any(|(i, _)| !word(line[..i].chars().next_back()));
            if opens || line.contains("refreshOpen") {
                calls.push(format!("{qml}: {line}"));
            }
        }
        assert!(!text.contains("Timer"), "{qml} holds a timer");
    }
    assert_eq!(calls, ["qml/Main.qml: dockNamed(name).open();", "qml/ProjectPane.qml: open(args[i]);", "qml/ProjectPane.qml: onClicked: projectTree.refreshOpen()"]);
    let bridge = fs::read_to_string(dir.join("src/shell/tree.rs")).expect("the bridge");
    for (call, count, what) in [
        ("read_folder(", 2, "where the view fetches a folder and where Refresh reads it"),
        ("refresh_folder(", 2, "defined, and called by Refresh alone"),
        ("refresh_open(", 2, "declared and defined, never called"),
        ("Tree::open(", 1, "one place opens a project"),
        (".expand(", 0, "nothing reads through the model's own reader"),
        (".refresh(", 0, "nothing refreshes through the model's own refresh"),
    ] {
        assert_eq!(bridge.matches(call).count(), count, "{call}: {what}");
    }
    let refresh_open = bridge.find("fn refresh_open(mut self").expect("Refresh's function");
    let call = bridge.find("self.as_mut().refresh_folder(").expect("its call");
    assert!(call > refresh_open && !bridge[refresh_open..call].contains("\n    fn "), "refresh_folder called inside refresh_open");
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
