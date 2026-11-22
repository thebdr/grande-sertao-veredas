//! A project folder as the workbench's tree (the spike's point 1, its Rust side): read one folder
//! at a time, folders first, every TOML file named by core's header reader and by the component
//! whose reader states its format, nothing followed, nothing run.

use std::fs;
use std::path::Path;

use oficina_spike_model::tree::{Contents, Entry, FileKind, NodeId, Refreshed, STAMP_READ_LIMIT, Tree, TreeError};
use veredas_core::schema;

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().expect("a parent")).expect("the folder");
    fs::write(p, text).expect("the file");
}

fn stamped(format: &str, version: u32) -> String {
    format!("format = \"{format}\"\nversion = {version}\n")
}

/// A project folder: a portal project, core's channel, a bench file with an accented name, a file
/// of a format no reader states yet, refused and broken TOML, a cargo manifest, a TOML file past
/// the read limit, other files, a hidden cargo configuration, and two folders.
fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("a scratch folder");
    let r = dir.path();
    write(r, "project.toml", &format!("{}name = \"the press hall\"\n", stamped("veredas-project", 1)));
    write(r, "channel.toml", &stamped("veredas-channel", 1));
    write(r, "ação.toml", &stamped("veredas-bench", 1));
    write(r, "later.toml", &stamped("veredas-panel", 3));
    write(r, "B.toml", "x = 1\n");
    write(r, "broken.toml", "x = [\n");
    write(r, "Cargo.toml", "[package]\nname = \"orcl-demo\"\n");
    write(r, "big.toml", &"#".repeat(STAMP_READ_LIMIT as usize + 1));
    write(r, "layout.svg", "<svg/>");
    write(r, "notes.txt", "notes");
    write(r, ".cargo/config.toml", "[build]\nrustc-wrapper = \"./wrapper.sh\"\n");
    write(r, "alpha/deep.toml", &stamped("veredas-binding", 1));
    fs::create_dir_all(r.join("alpha/inner")).expect("a folder");
    fs::create_dir_all(r.join("Zeta")).expect("a folder");
    dir
}

/// A node as one line: its name and what the tree says it is.
fn label(t: &Tree, id: NodeId) -> String {
    let n = t.node(id);
    let what = match &n.entry {
        Entry::Folder => "folder".to_string(),
        Entry::Link { target } => format!("link to {target}"),
        Entry::Special => "special".to_string(),
        Entry::File(FileKind::Stamped { format, version, component }) => {
            format!("{format} {version} of {}", component.map_or("no component", |c| c.display))
        }
        Entry::File(FileKind::Refused { why }) => format!("refused: {why}"),
        Entry::File(FileKind::TooLarge { bytes }) => format!("too large: {bytes} bytes"),
        Entry::File(FileKind::Unreadable { why }) => format!("unreadable: {why}"),
        Entry::File(FileKind::Manifest) => "cargo manifest".to_string(),
        Entry::File(FileKind::Other) => "file".to_string(),
    };
    format!("{}{} | {what}", if n.hidden { "(hidden) " } else { "" }, n.name)
}

fn rows(t: &Tree, id: NodeId) -> Vec<String> {
    t.children(id).expect("read").iter().map(|c| label(t, *c)).collect()
}

fn child(t: &Tree, id: NodeId, name: &str) -> NodeId {
    *t.children(id).expect("read").iter().find(|c| t.node(**c).name == name).unwrap_or_else(|| panic!("no {name}"))
}

/// Core's own words for a text it refuses.
fn refusal(text: &str) -> String {
    schema::stamp(text, "a TOML file").unwrap_err().to_string()
}

#[test]
fn a_project_folder_opens_one_level_deep_folders_first_each_toml_file_named_by_its_stamp() {
    let dir = project();
    let t = Tree::open(dir.path()).expect("the folder opens");
    assert_eq!(
        rows(&t, t.root()),
        vec![
            "(hidden) .cargo | folder".to_string(),
            "alpha | folder".to_string(),
            "Zeta | folder".to_string(),
            "ação.toml | veredas-bench 1 of Maquinar".to_string(),
            "B.toml | refused: no `format`: a TOML file states `format` and `version`".to_string(),
            format!("big.toml | too large: {} bytes", STAMP_READ_LIMIT + 1),
            format!("broken.toml | refused: {}", refusal("x = [\n")),
            "Cargo.toml | cargo manifest".to_string(),
            "channel.toml | veredas-channel 1 of Core".to_string(),
            "later.toml | veredas-panel 3 of no component".to_string(),
            "layout.svg | file".to_string(),
            "notes.txt | file".to_string(),
            "project.toml | veredas-project 1 of Portal".to_string(),
        ]
    );
    assert!(refusal("x = [\n").starts_with("not a TOML file: "), "{}", refusal("x = [\n"));
    // Each node knows its place and its path, and nothing below the root has been read.
    let alpha = child(&t, t.root(), "alpha");
    assert_eq!((t.row(alpha), t.node(alpha).parent), (Some(1), Some(t.root())));
    assert_eq!(t.path(child(&t, t.root(), "ação.toml")), dir.path().join("ação.toml"));
    assert_eq!(t.node(alpha).contents, Contents::NotRead);
    assert_eq!(t.children(alpha), None);
    assert_eq!(t.row(t.root()), None);
    // The cargo configuration is a refused TOML file like any other: shown, never honoured.
    let mut t = t;
    let cargo = child(&t, t.root(), ".cargo");
    t.expand(cargo).expect("it opens");
    assert_eq!(rows(&t, cargo), vec!["config.toml | refused: no `format`: a TOML file states `format` and `version`"]);
}

#[test]
fn a_folder_is_read_when_it_opens_and_once() {
    let dir = project();
    let mut t = Tree::open(dir.path()).expect("the folder opens");
    let alpha = child(&t, t.root(), "alpha");
    let first = t.expand(alpha).expect("alpha opens").to_vec();
    assert_eq!(rows(&t, alpha), vec!["inner | folder", "deep.toml | veredas-binding 1 of Supervisão"]);
    // A file written after the folder was read is not seen until the folder is read again.
    write(dir.path(), "alpha/late.toml", &stamped("veredas-catalog", 1));
    assert_eq!(t.expand(alpha).expect("again").to_vec(), first);
    let inner = child(&t, alpha, "inner");
    assert_eq!(t.children(inner), None);
    assert_eq!(t.expand(inner).expect("an empty folder").to_vec(), Vec::<NodeId>::new());
    assert_eq!(t.node(inner).contents, Contents::Read(Vec::new()));
    assert_eq!(t.path(child(&t, alpha, "deep.toml")), dir.path().join("alpha").join("deep.toml"));
    // A file is no folder to open.
    let deep = child(&t, alpha, "deep.toml");
    assert_eq!(
        t.expand(deep).unwrap_err(),
        TreeError::NotAFolder { path: dir.path().join("alpha").join("deep.toml").display().to_string() }
    );
}

#[test]
fn refresh_keeps_the_ids_of_what_stayed_and_says_what_left_and_what_came() {
    let dir = project();
    let r = dir.path();
    let mut t = Tree::open(r).expect("the folder opens");
    let root = t.root();
    let alpha = child(&t, root, "alpha");
    t.expand(alpha).expect("alpha opens");
    let deep = child(&t, alpha, "deep.toml");
    let b = child(&t, root, "B.toml");
    let notes = child(&t, root, "notes.txt");
    let zeta = child(&t, root, "Zeta");
    let project = child(&t, root, "project.toml");
    // A file comes, a file goes, a file gains a stamp, a folder becomes a file of its name.
    write(r, "new.toml", &stamped("veredas-catalog", 1));
    fs::remove_file(r.join("notes.txt")).expect("removed");
    write(r, "B.toml", &stamped("veredas-catalog", 1));
    fs::remove_dir(r.join("Zeta")).expect("removed");
    write(r, "Zeta", "now a file");
    let changes = t.refresh(root).expect("the folder reads again");
    let new = child(&t, root, "new.toml");
    let zeta_file = child(&t, root, "Zeta");
    assert_eq!(changes, Refreshed { removed: vec![zeta, notes], added: vec![new, zeta_file] });
    assert_eq!(
        rows(&t, root),
        vec![
            "(hidden) .cargo | folder",
            "alpha | folder",
            "ação.toml | veredas-bench 1 of Maquinar",
            "B.toml | veredas-catalog 1 of Core",
            &format!("big.toml | too large: {} bytes", STAMP_READ_LIMIT + 1),
            &format!("broken.toml | refused: {}", refusal("x = [\n")),
            "Cargo.toml | cargo manifest",
            "channel.toml | veredas-channel 1 of Core",
            "later.toml | veredas-panel 3 of no component",
            "layout.svg | file",
            "new.toml | veredas-catalog 1 of Core",
            "project.toml | veredas-project 1 of Portal",
            "Zeta | file",
        ]
    );
    // What stayed kept its id, a folder what was read of it; what left is gone for good.
    assert_eq!((child(&t, root, "B.toml"), child(&t, root, "alpha"), child(&t, root, "project.toml")), (b, alpha, project));
    assert_eq!(t.children(alpha).map(<[NodeId]>::to_vec), Some(vec![child(&t, alpha, "inner"), deep]));
    assert_eq!(t.expand(zeta).unwrap_err(), TreeError::Gone(zeta));
    assert_eq!(t.refresh(notes).unwrap_err(), TreeError::Gone(notes));
    // Nothing changed: nothing to say.
    assert_eq!(t.refresh(root).expect("again"), Refreshed::default());
    // A folder never read is read by its refresh, everything in it new.
    let cargo = child(&t, root, ".cargo");
    let first = t.refresh(cargo).expect("read");
    assert_eq!(first, Refreshed { removed: vec![], added: vec![child(&t, cargo, "config.toml")] });
}

#[test]
fn the_root_must_be_a_folder_that_reads() {
    let dir = project();
    let file = dir.path().join("notes.txt");
    assert_eq!(Tree::open(&file).unwrap_err(), TreeError::NotAFolder { path: file.display().to_string() });
    let missing = dir.path().join("missing");
    assert_eq!(
        Tree::open(&missing).unwrap_err(),
        TreeError::Read { path: missing.display().to_string(), why: "No such file or directory (os error 2)".into() }
    );
    assert_eq!(
        TreeError::NotAFolder { path: "x".into() }.to_string(),
        "`x` is not a folder"
    );
}

#[cfg(unix)]
mod unix {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{PermissionsExt, symlink};

    use super::*;

    #[test]
    fn a_link_is_shown_with_where_it_points_and_never_followed() {
        let dir = project();
        let r = dir.path();
        symlink("..", r.join("alpha/up")).expect("a link to the parent");
        symlink("../project.toml", r.join("alpha/proj.toml")).expect("a link to a file");
        symlink("nowhere", r.join("alpha/dangling")).expect("a dangling link");
        let mut t = Tree::open(r).expect("the folder opens");
        let alpha = child(&t, t.root(), "alpha");
        t.expand(alpha).expect("alpha opens");
        assert_eq!(
            rows(&t, alpha),
            vec![
                "inner | folder",
                "dangling | link to nowhere",
                "deep.toml | veredas-binding 1 of Supervisão",
                "proj.toml | link to ../project.toml",
                "up | link to ..",
            ]
        );
        let up = child(&t, alpha, "up");
        assert_eq!(t.expand(up).unwrap_err(), TreeError::NotAFolder { path: r.join("alpha/up").display().to_string() });
    }

    #[test]
    fn a_folder_or_a_file_that_cannot_be_read_says_why_and_the_rest_reads() {
        let dir = project();
        let r = dir.path();
        fs::set_permissions(r.join("Zeta"), fs::Permissions::from_mode(0o000)).expect("closed");
        fs::set_permissions(r.join("channel.toml"), fs::Permissions::from_mode(0o000)).expect("closed");
        if fs::read_dir(r.join("Zeta")).is_ok() {
            eprintln!("skipped: this user reads a folder whatever its mode");
            return;
        }
        let mut t = Tree::open(r).expect("the folder opens");
        assert_eq!(
            label(&t, child(&t, t.root(), "channel.toml")),
            "channel.toml | unreadable: Permission denied (os error 13)"
        );
        let zeta = child(&t, t.root(), "Zeta");
        assert_eq!(
            t.expand(zeta).unwrap_err(),
            TreeError::Read { path: r.join("Zeta").display().to_string(), why: "Permission denied (os error 13)".into() }
        );
        assert_eq!(t.node(zeta).contents, Contents::Unreadable("Permission denied (os error 13)".into()));
        let alpha = child(&t, t.root(), "alpha");
        assert_eq!(t.expand(alpha).expect("a sibling still opens").len(), 2);
        // Opened again once it reads.
        fs::set_permissions(r.join("Zeta"), fs::Permissions::from_mode(0o755)).expect("open");
        assert_eq!(t.expand(zeta).expect("now it reads").len(), 0);
        assert_eq!(t.node(zeta).contents, Contents::Read(Vec::new()));
    }

    #[test]
    fn a_name_that_is_not_utf8_is_shown_with_a_replacement_and_reached_exactly() {
        let dir = tempfile::tempdir().expect("a scratch folder");
        let name = OsStr::from_bytes(b"caf\xe9.toml");
        fs::write(dir.path().join(name), stamped("veredas-project", 1)).expect("the file");
        let t = Tree::open(dir.path()).expect("the folder opens");
        let id = t.children(t.root()).expect("read")[0];
        assert_eq!(label(&t, id), "caf\u{FFFD}.toml | veredas-project 1 of Portal");
        assert_eq!(t.path(id), dir.path().join(name));
        assert_eq!(fs::read_to_string(t.path(id)).expect("reached"), stamped("veredas-project", 1));
    }
}
