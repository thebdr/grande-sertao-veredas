//! C-290: the suite release's manifest. The plan's worked example read as its keys say; every
//! refusal of the reader by its message; a manifest checked against throwaway repositories'
//! history (a component left out, one too many, a version or an executable that differs, a commit
//! that is not there or not an ancestor, two releases pinning one version at different code, a
//! file named after another release); the executable on both outcomes; and every manifest of this
//! repository checked against its own history.

use std::path::{Path, PathBuf};
use std::process::Command;

use entrega::manifest::{self, Manifest};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The first `toml` block of the plan: the worked example.
fn example() -> String {
    let plan = std::fs::read_to_string(repo_root().join("docs/entrega-plan.md")).unwrap();
    let start = plan.find("```toml\n").expect("the plan holds a toml block") + "```toml\n".len();
    let end = start + plan[start..].find("```").unwrap();
    plan[start..end].to_string()
}

#[test]
fn the_plans_example_reads_as_its_keys_say() {
    let m = manifest::load(&example()).unwrap();
    assert_eq!(m.release, semver::Version::parse("2.0.0-alpha.1").unwrap());
    assert_eq!(m.commit, "0123456789abcdef0123456789abcdef01234567");
    assert_eq!(m.target, "x86_64-unknown-linux-gnu");
    assert_eq!(m.toolchain, "rustc 1.100.0 (0ef1bc386 2026-11-09)");
    let got: Vec<(String, String, Vec<String>)> =
        m.components.iter().map(|c| (c.name.clone(), c.version.to_string(), c.bins.clone())).collect();
    let want = |n: &str, b: &[&str]| (n.to_string(), "2.0.0-alpha.1".to_string(), b.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(
        got,
        vec![
            want("veredas-core", &[]),
            want("veredas-maquinar", &["veredas-maquinar"]),
            want("veredas-portal", &["veredas-portal"]),
            want("veredas-supervisao", &["veredas-supervisao"]),
        ]
    );
    assert_eq!(m.tested.suite, "cargo nextest run --workspace");
    assert_eq!(m.tested.together, "the bundle's bench and supervisão on its plant");
}

/// The example with one line replaced (`from` must occur once).
fn edited(from: &str, to: &str) -> String {
    let ex = example();
    assert_eq!(ex.matches(from).count(), 1, "`{from}` occurs once in the example");
    ex.replacen(from, to, 1)
}

fn refused(text: &str) -> String {
    match manifest::load(text) {
        Ok(m) => panic!("accepted: {m:?}"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn the_header_is_cores_and_refused_by_name() {
    assert_eq!(refused(&edited("format = \"veredas-release\"", "format = \"veredas-bench\"")), "`format` is `veredas-bench`, not `veredas-release`");
    assert_eq!(refused(&edited("version = 1\n", "version = 2\n")), "version 2 of `veredas-release` is not known here (this reader knows 1)");
    assert_eq!(refused(&edited("format = \"veredas-release\"\n", "")), "no `format`: a release manifest states `format` and `version`");
    assert_eq!(refused(&edited("version = 1\n", "")), "no `version`: a release manifest states `format` and `version`");
    assert!(refused("format = \"veredas-release\"\nversion = 1\nrelease = ").starts_with("not a release manifest: "));
}

#[test]
fn a_misspelled_or_missing_key_is_refused_wherever_it_sits() {
    let unknown = |t: &str, key: &str| {
        let e = refused(t);
        assert!(e.starts_with(&format!("a release manifest: unknown field `{key}`")), "{e}");
    };
    unknown(&edited("release = ", "relase = "), "relase");
    unknown(&edited("bins = [\"veredas-portal\"]", "bin = [\"veredas-portal\"]"), "bin");
    unknown(&edited("together = ", "togther = "), "togther");
    unknown(&edited("[tested]\n", "[tested]\nextra = 1\n"), "extra");
    let missing = |t: &str, key: &str| assert_eq!(refused(t), format!("a release manifest: missing field `{key}`"));
    missing(&edited("commit = \"0123456789abcdef0123456789abcdef01234567\"\n", ""), "commit");
    missing(&edited("target = \"x86_64-unknown-linux-gnu\"\n", ""), "target");
    missing(&edited("toolchain = \"rustc 1.100.0 (0ef1bc386 2026-11-09)\"\n", ""), "toolchain");
    let ex = example();
    let no_tested = &ex[..ex.find("[tested]").unwrap()];
    missing(no_tested, "tested");
    missing(&edited("suite = \"cargo nextest run --workspace\"\n", ""), "suite");
}

#[test]
fn every_value_is_checked_for_its_form() {
    let r = |from: &str, to: &str| refused(&edited(from, to));
    let p = "a release manifest: ";
    assert_eq!(r("release = \"2.0.0-alpha.1\"", "release = \"2.0\""), format!("{p}`release` is `2.0`, not SemVer"));
    let c = "0123456789abcdef0123456789abcdef01234567";
    for bad in ["0123456789abcdef", "0123456789ABCDEF0123456789ABCDEF01234567", "0123456789abcdef0123456789abcdef0123456g", "0123456789abcdef0123456789abcdef012345678"] {
        assert_eq!(
            r(&format!("commit = \"{c}\""), &format!("commit = \"{bad}\"")),
            format!("{p}`commit` is `{bad}`, not a full commit id (40 lowercase hexadecimal characters)")
        );
    }
    assert_eq!(r("target = \"x86_64-unknown-linux-gnu\"", "target = \"\""), format!("{p}`target` is empty"));
    assert_eq!(r("target = \"x86_64-unknown-linux-gnu\"", "target = \"x86_64 linux\""), format!("{p}`target` is `x86_64 linux`, not a target triple"));
    assert_eq!(r("toolchain = \"rustc 1.100.0 (0ef1bc386 2026-11-09)\"", "toolchain = \"  \""), format!("{p}`toolchain` is empty"));
    assert_eq!(r("toolchain = \"rustc 1.100.0 (0ef1bc386 2026-11-09)\"", "toolchain = \" rustc\""), format!("{p}`toolchain` begins or ends with whitespace"));
    assert_eq!(r("suite = \"cargo nextest run --workspace\"", "suite = \"\""), format!("{p}`tested.suite` is empty"));
    assert_eq!(r("together = \"the bundle's bench and supervisão on its plant\"", "together = \"x \""), format!("{p}`tested.together` begins or ends with whitespace"));
}

#[test]
fn the_components_are_suite_crates_each_once_with_their_executables_once() {
    let ex = example();
    let p = "a release manifest: ";
    let only_tested = format!("{}{}", &ex[..ex.find("[[component]]").unwrap()], &ex[ex.find("[tested]").unwrap()..]);
    assert_eq!(refused(&only_tested), format!("{p}no `[[component]]`: a release pins at least one component"));
    for bad in ["portal", "veredas-Portal", "veredas_portal", "veredas-", "veredas-2x", "leis", "orcl-"] {
        assert_eq!(
            refused(&edited("name = \"veredas-portal\"", &format!("name = \"{bad}\""))),
            format!("{p}component `{bad}` is not a suite crate (veredas-<nome> or orcl-<nome>)")
        );
    }
    assert!(manifest::load(&edited("name = \"veredas-portal\"", "name = \"orcl-std\"")).is_ok());
    assert_eq!(refused(&edited("name = \"veredas-portal\"", "name = \"veredas-core\"")), format!("{p}component `veredas-core` is listed twice"));
    assert_eq!(
        refused(&edited("name = \"veredas-portal\"\nversion = \"2.0.0-alpha.1\"", "name = \"veredas-portal\"\nversion = \"2\"")),
        format!("{p}component `veredas-portal`: `version` is `2`, not SemVer")
    );
    for bad in ["bin/veredas-portal", "", "a b", "../x"] {
        assert_eq!(
            refused(&edited("bins = [\"veredas-portal\"]", &format!("bins = [\"{bad}\"]"))),
            format!("{p}component `veredas-portal`: bin `{bad}` is not an executable's name")
        );
    }
    assert_eq!(
        refused(&edited("bins = [\"veredas-portal\"]", "bins = [\"veredas-portal\", \"veredas-portal\"]")),
        format!("{p}component `veredas-portal`: bin `veredas-portal` is listed twice")
    );
}

/// A throwaway repository: files written and committed on `main`, removed on drop.
struct Repo {
    dir: tempfile::TempDir,
}

impl Repo {
    fn new() -> Repo {
        let r = Repo { dir: tempfile::tempdir().unwrap() };
        r.git(&["init", "-q", "-b", "main"]);
        r
    }
    fn path(&self) -> &Path {
        self.dir.path()
    }
    fn git(&self, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(self.path())
            .args(["-c", "user.name=entrega", "-c", "user.email=entrega@example.invalid", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    }
    fn write(&self, rel: &str, text: &str) {
        let p = self.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }
    fn commit(&self, msg: &str) -> String {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", msg]);
        self.git(&["rev-parse", "HEAD"])
    }
}

const WORKSPACE: &str = "[workspace]\nmembers = [\"tools/ferramenta\", \"crates/veredas-core\", \"crates/veredas-maquinar\", \"crates/veredas-portal\"]\n";

/// A suite of three components and a tool: core a library, the bench an executable by
/// `src/main.rs`, portal one by a `[[bin]]` table and one by `src/bin/`.
fn suite() -> Repo {
    let r = Repo::new();
    r.write("Cargo.toml", WORKSPACE);
    r.write("tools/ferramenta/Cargo.toml", "[package]\nname = \"ferramenta\"\nversion = \"0.1.0\"\n");
    r.write("tools/ferramenta/src/main.rs", "fn main() {}\n");
    r.write("crates/veredas-core/Cargo.toml", "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\n");
    r.write("crates/veredas-core/src/lib.rs", "\n");
    r.write("crates/veredas-maquinar/Cargo.toml", "[package]\nname = \"veredas-maquinar\"\nversion = \"2.1.0\"\n");
    r.write("crates/veredas-maquinar/src/lib.rs", "\n");
    r.write("crates/veredas-maquinar/src/main.rs", "fn main() {}\n");
    r.write(
        "crates/veredas-portal/Cargo.toml",
        "[package]\nname = \"veredas-portal\"\nversion = \"2.0.0-alpha.3\"\n\n[[bin]]\nname = \"veredas-portal\"\npath = \"src/main.rs\"\n",
    );
    r.write("crates/veredas-portal/src/main.rs", "fn main() {}\n");
    r.write("crates/veredas-portal/src/bin/porteiro.rs", "fn main() {}\n");
    r
}

/// The manifest that matches [`suite`] at `commit`, as text.
fn matching(release: &str, commit: &str) -> String {
    format!(
        "format = \"veredas-release\"\nversion = 1\nrelease = \"{release}\"\ncommit = \"{commit}\"\ntarget = \"x86_64-unknown-linux-gnu\"\ntoolchain = \"rustc 1.100.0\"\n\n\
         [[component]]\nname = \"veredas-core\"\nversion = \"2.0.0\"\n\n\
         [[component]]\nname = \"veredas-maquinar\"\nversion = \"2.1.0\"\nbins = [\"veredas-maquinar\"]\n\n\
         [[component]]\nname = \"veredas-portal\"\nversion = \"2.0.0-alpha.3\"\nbins = [\"porteiro\", \"veredas-portal\"]\n\n\
         [tested]\nsuite = \"cargo nextest run --workspace\"\ntogether = \"the bundle's bench and supervisão on its plant\"\n"
    )
}

fn checked(r: &Repo, text: &str) -> Vec<String> {
    manifest::check(r.path(), &manifest::load(text).unwrap()).unwrap()
}

#[test]
fn a_manifest_that_matches_its_commit_checks() {
    let r = suite();
    let c = r.commit("the suite");
    assert_eq!(checked(&r, &matching("2.0.0", &c)), Vec::<String>::new());
    let held = manifest::components_at(r.path(), &c, &mut Vec::new()).unwrap();
    let names: Vec<&String> = held.keys().collect();
    assert_eq!(names, ["veredas-core", "veredas-maquinar", "veredas-portal"], "the tool is no component");
    // Later commits do not move it: the manifest is judged at its own commit.
    r.write("crates/veredas-portal/Cargo.toml", "[package]\nname = \"veredas-portal\"\nversion = \"2.0.0\"\n");
    r.commit("portal moves on");
    assert_eq!(checked(&r, &matching("2.0.0", &c)), Vec::<String>::new());
}

#[test]
fn a_component_left_out_or_one_too_many_is_refused() {
    let r = suite();
    let c = r.commit("the suite");
    let text = matching("2.0.0", &c);
    let without_portal = format!("{}[tested]{}", &text[..text.find("[[component]]\nname = \"veredas-portal\"").unwrap()], text.split("[tested]").nth(1).unwrap());
    assert_eq!(checked(&r, &without_portal), vec![format!("veredas-portal 2.0.0-alpha.3 is a component at {c} and the manifest leaves it out")]);
    let with_aranha = text.replace("[tested]", "[[component]]\nname = \"veredas-aranha\"\nversion = \"2.0.0\"\n\n[tested]");
    assert_eq!(checked(&r, &with_aranha), vec![format!("veredas-aranha is no component at {c}")]);
}

#[test]
fn a_version_or_an_executable_that_differs_is_refused() {
    let r = suite();
    let c = r.commit("the suite");
    let text = matching("2.0.0", &c);
    assert_eq!(
        checked(&r, &text.replace("version = \"2.1.0\"", "version = \"2.1.1\"")),
        vec![format!("veredas-maquinar is 2.1.0 at {c}, not 2.1.1")]
    );
    assert_eq!(
        checked(&r, &text.replace("bins = [\"porteiro\", \"veredas-portal\"]", "bins = [\"veredas-portal\"]")),
        vec![format!("veredas-portal builds `porteiro` at {c} and the manifest does not ship it")]
    );
    assert_eq!(
        checked(&r, &text.replace("version = \"2.0.0\"\n\n[[component]]\nname = \"veredas-maquinar\"", "version = \"2.0.0\"\nbins = [\"veredas-core\"]\n\n[[component]]\nname = \"veredas-maquinar\"")),
        vec![format!("veredas-core builds no `veredas-core` at {c}")]
    );
    // `autobins = false` keeps `src/main.rs` and `src/bin/` out, as cargo does.
    r.write(
        "crates/veredas-portal/Cargo.toml",
        "[package]\nname = \"veredas-portal\"\nversion = \"2.0.0-alpha.3\"\nautobins = false\n\n[[bin]]\nname = \"veredas-portal\"\npath = \"src/main.rs\"\n",
    );
    let c2 = r.commit("no autobins");
    assert_eq!(
        checked(&r, &matching("2.0.0", &c2)),
        vec![format!("veredas-portal builds no `porteiro` at {c2}")]
    );
    // A folder under src/bin/ with a main.rs is an executable named after the folder.
    r.write("crates/veredas-maquinar/src/bin/bancada/main.rs", "fn main() {}\n");
    let c3 = r.commit("a bin folder");
    assert_eq!(
        checked(&r, &matching("2.0.0", &c3)),
        vec![
            format!("veredas-maquinar builds `bancada` at {c3} and the manifest does not ship it"),
            format!("veredas-portal builds no `porteiro` at {c3}"),
        ]
    );
}

#[test]
fn a_commit_not_there_or_not_an_ancestor_is_refused() {
    let r = suite();
    let c = r.commit("the suite");
    let absent = "0123456789abcdef0123456789abcdef01234567";
    assert_eq!(checked(&r, &matching("2.0.0", absent)), vec![format!("commit {absent} is not in the repository")]);
    r.git(&["checkout", "-q", "-b", "side"]);
    r.write("docs/side.md", "a side branch\n");
    let side = r.commit("on a side branch");
    r.git(&["checkout", "-q", "main"]);
    assert_eq!(
        checked(&r, &matching("2.0.0", &side)),
        vec![format!("commit {side} is not an ancestor of HEAD: a release is cut from the history it is committed in")]
    );
    assert_eq!(checked(&r, &matching("2.0.0", &c)), Vec::<String>::new());
}

#[test]
fn a_version_names_one_tree_across_releases() {
    let r = suite();
    let c1 = r.commit("the suite");
    r.write("docs/notes.md", "nothing of the components\n");
    let c2 = r.commit("docs alone");
    r.write("crates/veredas-portal/src/main.rs", "fn main() { println!(\"changed\"); }\n");
    let c3 = r.commit("portal changed, its version not bumped");
    let load = |rel: &str, c: &str| (PathBuf::from(rel), manifest::load(&matching(rel, c)).unwrap());
    let same: Vec<(PathBuf, Manifest)> = vec![load("2.0.0", &c1), load("2.0.1", &c2)];
    assert_eq!(manifest::check_together(r.path(), &same).unwrap(), Vec::<String>::new());
    let moved: Vec<(PathBuf, Manifest)> = vec![load("2.0.0", &c1), load("2.0.1", &c2), load("2.0.2", &c3)];
    assert_eq!(
        manifest::check_together(r.path(), &moved).unwrap(),
        vec!["veredas-portal 2.0.0-alpha.3 is pinned by releases 2.0.0 and 2.0.2 at different code: a version names one tree".to_string()]
    );
}

#[test]
fn a_releases_folder_holds_manifests_named_after_their_release() {
    let r = suite();
    let c = r.commit("the suite");
    let dir = r.path().join("releases");
    assert!(manifest::load_dir(&dir).unwrap().is_empty(), "no folder, no release");
    r.write("releases/2.0.0.toml", &matching("2.0.0", &c));
    let loaded = manifest::load_dir(&dir).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].1.release.to_string(), "2.0.0");
    r.write("releases/2.0.1.toml", &matching("2.0.2", &c));
    r.write("releases/notes.md", "not a manifest\n");
    let refusals = manifest::load_dir(&dir).unwrap_err();
    assert_eq!(
        refusals,
        vec![
            format!("{}: the file is named `2.0.1.toml`, not after its release 2.0.2", dir.join("2.0.1.toml").display()),
            format!("{}: a releases folder holds manifests (`<release>.toml`) alone", dir.join("notes.md").display()),
        ]
    );
}

#[test]
fn the_executable_checks_a_repositorys_releases_and_says_why_one_is_refused() {
    let r = suite();
    let c = r.commit("the suite");
    r.write("releases/2.0.0.toml", &matching("2.0.0", &c));
    r.commit("release 2.0.0");
    let run = || Command::new(env!("CARGO_BIN_EXE_entrega")).arg("check").current_dir(r.path().join("crates")).output().unwrap();
    let out = run();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{}: release 2.0.0 at {}, 3 components, checked\n", r.path().canonicalize().unwrap().join("releases/2.0.0.toml").display(), &c[..12])
    );
    r.write("releases/2.0.0.toml", &matching("2.0.0", &c).replace("version = \"2.1.0\"", "version = \"2.1.1\""));
    let out = run();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        format!("{}: veredas-maquinar is 2.1.0 at {c}, not 2.1.1\n", r.path().canonicalize().unwrap().join("releases/2.0.0.toml").display())
    );
    let usage = Command::new(env!("CARGO_BIN_EXE_entrega")).output().unwrap();
    assert_eq!(usage.status.code(), Some(1));
    assert_eq!(String::from_utf8(usage.stderr).unwrap(), "usage: entrega check [<manifest.toml>...]\n");
}

#[test]
fn every_release_of_this_repository_checks_against_its_history() {
    let root = repo_root().canonicalize().unwrap();
    let all = manifest::load_dir(&root.join("releases")).unwrap();
    for (p, m) in &all {
        assert_eq!(manifest::check(&root, m).unwrap(), Vec::<String>::new(), "{}", p.display());
    }
    assert_eq!(manifest::check_together(&root, &all).unwrap(), Vec::<String>::new());
}
