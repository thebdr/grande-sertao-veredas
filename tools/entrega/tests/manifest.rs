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
    r.write("crates/veredas-core/Cargo.toml", "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\nedition = \"2024\"\n");
    r.write("crates/veredas-core/src/lib.rs", "\n");
    r.write("crates/veredas-maquinar/Cargo.toml", "[package]\nname = \"veredas-maquinar\"\nversion = \"2.1.0\"\nedition = \"2024\"\n");
    r.write("crates/veredas-maquinar/src/lib.rs", "\n");
    r.write("crates/veredas-maquinar/src/main.rs", "fn main() {}\n");
    r.write(
        "crates/veredas-portal/Cargo.toml",
        "[package]\nname = \"veredas-portal\"\nversion = \"2.0.0-alpha.3\"\nedition = \"2024\"\n\n[[bin]]\nname = \"veredas-portal\"\npath = \"src/main.rs\"\n",
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
    r.write("crates/veredas-portal/Cargo.toml", "[package]\nname = \"veredas-portal\"\nversion = \"2.0.0\"\nedition = \"2024\"\n");
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
    // `autobins = false` keeps `src/bin/` out, as cargo reads it.
    r.write(
        "crates/veredas-portal/Cargo.toml",
        "[package]\nname = \"veredas-portal\"\nversion = \"2.0.0-alpha.3\"\nedition = \"2024\"\nautobins = false\n\n[[bin]]\nname = \"veredas-portal\"\npath = \"src/main.rs\"\n",
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
fn the_executables_are_the_ones_cargo_builds_and_the_members_as_cargo_spells_them() {
    let r = Repo::new();
    // Members spelled with `./` and with a trailing slash, as cargo (and L1) accepts them.
    r.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"./crates/veredas-portal\", \"crates/veredas-maquinar/\", \"crates/veredas-core\", \"crates/veredas-velho\"]\n",
    );
    // A bin claiming src/main.rs under another name: cargo builds that name alone.
    r.write("crates/veredas-portal/Cargo.toml", "[package]\nname = \"veredas-portal\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[[bin]]\nname = \"porteiro\"\npath = \"src/main.rs\"\n");
    r.write("crates/veredas-portal/src/main.rs", "fn main() {}\n");
    // A bin named `a` at src/bin/b.rs: cargo builds `a`, never a `b` beside it.
    r.write("crates/veredas-maquinar/Cargo.toml", "[package]\nname = \"veredas-maquinar\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[[bin]]\nname = \"a\"\npath = \"src/bin/b.rs\"\n");
    r.write("crates/veredas-maquinar/src/lib.rs", "\n");
    r.write("crates/veredas-maquinar/src/bin/b.rs", "fn main() {}\n");
    // Required features change nothing here: every bin target cargo names is the component's
    // (whether cargo builds it with the defaults is the cut's to see, C-291).
    r.write(
        "crates/veredas-core/Cargo.toml",
        "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[features]\ndefault = [\"full\"]\nfull = [\"serve\"]\nserve = []\nextra = []\n\n\
         [[bin]]\nname = \"servido\"\npath = \"src/bin/servido.rs\"\nrequired-features = [\"serve\"]\n\n\
         [[bin]]\nname = \"extra\"\npath = \"src/bin/extra.rs\"\nrequired-features = [\"extra\"]\n",
    );
    r.write("crates/veredas-core/src/lib.rs", "\n");
    r.write("crates/veredas-core/src/bin/servido.rs", "fn main() {}\n");
    r.write("crates/veredas-core/src/bin/extra.rs", "fn main() {}\n");
    // Edition 2015: an explicit bin turns discovery of src/bin/ off.
    r.write("crates/veredas-velho/Cargo.toml", "[package]\nname = \"veredas-velho\"\nversion = \"2.0.0\"\n\n[[bin]]\nname = \"velho\"\npath = \"src/main.rs\"\n");
    r.write("crates/veredas-velho/src/main.rs", "fn main() {}\n");
    r.write("crates/veredas-velho/src/bin/outro.rs", "fn main() {}\n");
    let c = r.commit("cargo's own rules");
    let mut refusals = Vec::new();
    let held = manifest::components_at(r.path(), &c, &mut refusals).unwrap();
    assert_eq!(refusals, Vec::<String>::new());
    let got: Vec<(String, String, Vec<String>)> =
        held.values().map(|h| (h.name.clone(), h.dir.clone(), h.bins.iter().cloned().collect())).collect();
    let want = |n: &str, b: &[&str]| (n.to_string(), format!("crates/{n}"), b.iter().map(|x| x.to_string()).collect::<Vec<_>>());
    assert_eq!(
        got,
        vec![
            want("veredas-core", &["extra", "servido"]),
            want("veredas-maquinar", &["a"]),
            want("veredas-portal", &["porteiro"]),
            want("veredas-velho", &["velho"]),
        ]
    );
    // A workspace cargo cannot read is refused with cargo's reason.
    r.write("crates/veredas-velho/Cargo.toml", "[package]\nname = 3\n");
    let c2 = r.commit("a manifest cargo refuses");
    let mut refusals = Vec::new();
    manifest::components_at(r.path(), &c2, &mut refusals).unwrap();
    assert_eq!(refusals.len(), 1);
    assert!(refusals[0].starts_with(&format!("commit {c2}: cargo cannot read its workspace: error")), "{refusals:?}");
    assert!(refusals[0].contains("invalid type: integer `3`, expected a string"), "cargo's reason: {refusals:?}");
    assert!(!refusals[0].contains("entrega-tree-"), "no scratch path in the reason: {refusals:?}");
    // A member the commit does not hold: cargo's cause, not only its first line.
    std::fs::remove_dir_all(r.path().join("crates/veredas-velho")).unwrap();
    let c3 = r.commit("a member gone");
    let mut refusals = Vec::new();
    manifest::components_at(r.path(), &c3, &mut refusals).unwrap();
    assert_eq!(refusals.len(), 1);
    assert!(refusals[0].contains("crates/veredas-velho") && refusals[0].contains("No such file or directory"), "{refusals:?}");
}

#[test]
fn settings_outside_the_history_change_nothing_that_is_read() {
    let r = suite();
    r.write(".gitattributes", "crates/veredas-maquinar/Cargo.toml filter=muda\n");
    let c = r.commit("the suite, a filter named in the tree");
    let mut refusals = Vec::new();
    let plain = manifest::components_at(r.path(), &c, &mut refusals).unwrap();
    assert_eq!(refusals, Vec::<String>::new());
    let porteiro: Vec<&String> = plain["veredas-portal"].bins.iter().collect();
    assert_eq!(porteiro, ["porteiro", "veredas-portal"]);
    // Export rules in the repository's own attributes, a local attributes file, a smudge filter:
    // none in the history, none changing what is read.
    r.write(".git/info/attributes", "crates/veredas-portal/src/bin/** export-ignore\ncrates/veredas-core export-ignore\n");
    let global = r.path().join(".git/elsewhere-attributes");
    std::fs::write(&global, "*.rs export-ignore\n").unwrap();
    r.git(&["config", "core.attributesFile", &global.display().to_string()]);
    r.git(&["config", "filter.muda.smudge", "sed s/2.1.0/9.9.9/"]);
    let mut refusals = Vec::new();
    assert_eq!(manifest::components_at(r.path(), &c, &mut refusals).unwrap(), plain);
    assert_eq!(refusals, Vec::<String>::new());
    let text = matching("2.0.0", &c);
    assert_eq!(checked(&r, &text), Vec::<String>::new());
    assert_eq!(
        checked(&r, &text.replace("bins = [\"porteiro\", \"veredas-portal\"]", "bins = [\"veredas-portal\"]")),
        vec![format!("veredas-portal builds `porteiro` at {c} and the manifest does not ship it")]
    );
    // The export itself writes the files as stored, an executable's mode kept.
    let out = tempfile::tempdir().unwrap();
    std::fs::set_permissions(r.path().join("crates/veredas-maquinar/src/main.rs"), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let c2 = r.commit("an executable file");
    let n = entrega::git::export_tree(r.path(), &c2, &["crates/veredas-maquinar"], out.path()).unwrap();
    assert_eq!(n, 3);
    let m = out.path().join("crates/veredas-maquinar");
    assert_eq!(std::fs::read_to_string(m.join("Cargo.toml")).unwrap(), "[package]\nname = \"veredas-maquinar\"\nversion = \"2.1.0\"\nedition = \"2024\"\n");
    assert_eq!(std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(m.join("src/main.rs")).unwrap().permissions()) & 0o777, 0o755);
    assert!(!out.path().join("crates/veredas-portal").exists(), "only the paths asked for");
}

#[test]
fn a_scratch_folder_reached_through_a_link_and_calls_at_once_read_the_same() {
    let r = suite();
    let c = r.commit("the suite");
    let mut refusals = Vec::new();
    let plain = manifest::components_at(r.path(), &c, &mut refusals).unwrap();
    assert_eq!((plain.len(), refusals.len()), (3, 0));
    let real = tempfile::tempdir().unwrap();
    let holder = tempfile::tempdir().unwrap();
    let link = holder.path().join("link");
    std::os::unix::fs::symlink(real.path(), &link).unwrap();
    let mut refusals = Vec::new();
    assert_eq!(manifest::components_at_in(r.path(), &c, &link, &mut refusals).unwrap(), plain);
    assert_eq!(refusals, Vec::<String>::new());
    std::thread::scope(|scope| {
        let calls: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    let mut refusals = Vec::new();
                    let held = manifest::components_at(r.path(), &c, &mut refusals).unwrap();
                    (held, refusals)
                })
            })
            .collect();
        for call in calls {
            let (held, refusals) = call.join().unwrap();
            assert_eq!((held, refusals), (plain.clone(), Vec::<String>::new()));
        }
    });
    assert_eq!(std::fs::read_dir(real.path()).unwrap().count(), 0, "no scratch folder left behind");
}

#[test]
fn replace_refs_and_grafts_change_neither_objects_nor_ancestry() {
    let r = suite();
    let c = r.commit("the suite");
    let text = matching("2.0.0", &c);
    let core_blob = r.git(&["rev-parse", &format!("{c}:crates/veredas-core/Cargo.toml")]);
    r.write("other-core.toml", "[package]\nname = \"veredas-core\"\nversion = \"9.9.9\"\nedition = \"2024\"\n\n[[bin]]\nname = \"intruso\"\npath = \"src/lib.rs\"\n");
    let fake = r.git(&["hash-object", "-w", "other-core.toml"]);
    std::fs::remove_file(r.path().join("other-core.toml")).unwrap();
    r.git(&["replace", &core_blob, &fake]);
    assert!(r.git(&["show", &format!("{c}:crates/veredas-core/Cargo.toml")]).contains("9.9.9"), "git itself reads the replacement");
    assert_eq!(checked(&r, &text), Vec::<String>::new(), "the true manifest still checks");
    assert_eq!(
        checked(&r, &text.replace("name = \"veredas-core\"\nversion = \"2.0.0\"", "name = \"veredas-core\"\nversion = \"9.9.9\"\nbins = [\"intruso\"]")),
        vec![format!("veredas-core is 2.0.0 at {c}, not 9.9.9"), format!("veredas-core builds no `intruso` at {c}")]
    );
    r.git(&["replace", "-d", &core_blob]);
    // A side commit made to look like an ancestor, by a replace graft and by the grafts file.
    r.git(&["checkout", "-q", "-b", "side"]);
    r.write("docs/side.md", "a side branch\n");
    let side = r.commit("on a side branch");
    r.git(&["checkout", "-q", "main"]);
    r.write("docs/later.md", "later\n");
    let later = r.commit("later on main");
    let not_ancestor = vec![format!("commit {side} is not an ancestor of HEAD: a release is cut from the history it is committed in")];
    r.git(&["replace", "--graft", &later, &c, &side]);
    assert_eq!(checked(&r, &matching("2.0.0", &side)), not_ancestor);
    r.git(&["replace", "-d", &later]);
    r.write(".git/info/grafts", &format!("{later} {c} {side}\n"));
    assert_eq!(checked(&r, &matching("2.0.0", &side)), not_ancestor);
}

#[test]
fn a_tree_holding_a_link_or_a_name_that_is_not_utf8_is_refused_by_its_path() {
    let refused_at = |r: &Repo, c: &str| {
        let mut refusals = Vec::new();
        let held = manifest::components_at(r.path(), c, &mut refusals).unwrap();
        assert!(held.is_empty());
        refusals
    };
    let link = |r: &Repo, at: &str, to: &str| {
        let p = r.path().join(at);
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_dir_all(&p);
        std::os::unix::fs::symlink(to, p).unwrap();
    };
    // An absolute link, and one climbing out.
    for to in ["/etc/hostname", "../../../outside/Cargo.toml"] {
        let r = suite();
        link(&r, "crates/veredas-core/Cargo.toml", to);
        let c = r.commit("a link out of the tree");
        assert_eq!(refused_at(&r, &c), vec![format!("commit {c}: the link `crates/veredas-core/Cargo.toml`: a release's tree holds no link, which could lead outside the history")]);
    }
    // A pair of links that, read as text, stays inside and, on disk, reaches out.
    let r = suite();
    link(&r, "crates/veredas-core/r", "../..");
    link(&r, "crates/veredas-core/Cargo.toml", "r/../../outside.toml");
    let c = r.commit("a chained pair of links");
    assert_eq!(refused_at(&r, &c), vec![format!("commit {c}: the link `crates/veredas-core/Cargo.toml`: a release's tree holds no link, which could lead outside the history")]);
    // A member folder that is itself a link.
    let r = suite();
    std::fs::rename(r.path().join("crates/veredas-portal"), r.path().join("fonte")).unwrap();
    link(&r, "crates/veredas-portal", "../fonte");
    let c = r.commit("a member folder as a link");
    assert_eq!(refused_at(&r, &c), vec![format!("commit {c}: the link `crates/veredas-portal`: a release's tree holds no link, which could lead outside the history")]);
    // A file name that is not UTF-8.
    let r = suite();
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::OsStr::from_bytes(b"\xff.rs");
    std::fs::write(r.path().join("crates/veredas-core/src").join(name), "fn main() {}\n").unwrap();
    let c = r.commit("a name that is not UTF-8");
    let refusals = refused_at(&r, &c);
    assert_eq!(refusals.len(), 1);
    assert!(refusals[0].starts_with(&format!("commit {c}: the path `crates/veredas-core/src/")) && refusals[0].ends_with("` is not UTF-8: a release's tree names its files in UTF-8"), "{refusals:?}");
}

#[test]
fn nothing_outside_the_commits_tree_is_read_or_written() {
    // A path dependency reaching out of the tree, refused whether or not the file it names exists.
    let r = suite();
    r.write(
        "crates/veredas-core/Cargo.toml",
        "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[dependencies]\nfora = { path = \"../../../../fora\" }\n",
    );
    let c = r.commit("a path dependency out of the tree");
    for planted in [false, true] {
        let parent = tempfile::tempdir().unwrap();
        if planted {
            std::fs::create_dir_all(parent.path().join("fora/src")).unwrap();
            std::fs::write(parent.path().join("fora/Cargo.toml"), "[package]\nname = \"fora\"\nversion = \"1.0.0\"\nedition = \"2024\"\n").unwrap();
            std::fs::write(parent.path().join("fora/src/lib.rs"), "\n").unwrap();
        }
        let mut refusals = Vec::new();
        let held = manifest::components_at_in(r.path(), &c, parent.path(), &mut refusals).unwrap();
        assert!(held.is_empty(), "planted: {planted}");
        assert_eq!(refusals.len(), 1, "planted: {planted}: {refusals:?}");
        if planted {
            assert_eq!(refusals[0], format!("commit {c}: the package `veredas-core` depends on `fora` by a path outside the commit's tree"));
        } else {
            assert!(refusals[0].starts_with(&format!("commit {c}: cargo cannot read its workspace: ")), "{refusals:?}");
        }
    }
    // A root that is no workspace: refused before cargo looks above the tree.
    let r = Repo::new();
    r.write("Cargo.toml", "[package]\nname = \"veredas-raiz\"\nversion = \"2.0.0\"\n");
    r.write("src/lib.rs", "\n");
    let c = r.commit("a package at the root");
    let parent = tempfile::tempdir().unwrap();
    std::fs::write(parent.path().join("Cargo.toml"), "[workspace]\nmembers = [\"entrega-tree-*/tree\", \"crates/*\"]\n").unwrap();
    let mut refusals = Vec::new();
    assert!(manifest::components_at_in(r.path(), &c, parent.path(), &mut refusals).unwrap().is_empty());
    assert_eq!(refusals, vec![format!("commit {c}: its Cargo.toml is no workspace (`[workspace]`): cargo would look for one above the tree")]);
    // A tree whose entry is `..` (git mktree makes one): refused, nothing written outside.
    let r = suite();
    let c = r.commit("the suite");
    std::fs::write(r.path().join("planted.txt"), "planted\n").unwrap();
    let blob = r.git(&["hash-object", "-w", "planted.txt"]);
    std::fs::remove_file(r.path().join("planted.txt")).unwrap();
    let mktree = |listing: String| -> String {
        let mut child = Command::new("git").arg("-C").arg(r.path()).arg("mktree").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
        use std::io::Write;
        child.stdin.take().unwrap().write_all(listing.as_bytes()).unwrap();
        String::from_utf8(child.wait_with_output().unwrap().stdout).unwrap().trim().to_string()
    };
    let inner = mktree(format!("100644 blob {blob}\tplanted.txt\n"));
    let up = mktree(format!("040000 tree {inner}\t..\n"));
    let root_listing = r.git(&["ls-tree", &c]);
    let root = mktree(format!("{root_listing}\n040000 tree {up}\t..\n"));
    let escape = r.git(&["commit-tree", &root, "-p", &c, "-m", "a dot-dot entry"]);
    let parent = tempfile::tempdir().unwrap();
    let mut refusals = Vec::new();
    assert!(manifest::components_at_in(r.path(), &escape, parent.path(), &mut refusals).unwrap().is_empty());
    assert_eq!(refusals, vec![format!("commit {escape}: the path `../../planted.txt` has a `..`, `.` or empty part: it would be written outside its place")]);
    assert_eq!(std::fs::read_dir(parent.path()).unwrap().count(), 0, "nothing written, nothing left");
}

#[test]
fn a_release_whose_commit_is_absent_hides_no_other_refusal() {
    let r = suite();
    let c1 = r.commit("the suite");
    r.write("crates/veredas-portal/src/main.rs", "fn main() { println!(\"changed\"); }\n");
    let c2 = r.commit("portal changed, its version not bumped");
    let load = |rel: &str, c: &str| (PathBuf::from(rel), manifest::load(&matching(rel, c)).unwrap());
    let absent = "0123456789abcdef0123456789abcdef01234567";
    let all: Vec<(PathBuf, Manifest)> = vec![load("2.0.0", &c1), load("2.0.1", &c2), load("2.0.2", absent)];
    assert_eq!(
        manifest::check_together(r.path(), &all).unwrap(),
        vec!["veredas-portal 2.0.0-alpha.3 is pinned by releases 2.0.0 and 2.0.1 at different code: a version names one tree".to_string()]
    );
    // Through the executable, every refusal said: the absent commit's and the one tree's.
    for (rel, c) in [("2.0.0", c1.as_str()), ("2.0.1", c2.as_str()), ("2.0.2", absent)] {
        r.write(&format!("releases/{rel}.toml"), &matching(rel, c));
    }
    r.commit("three releases");
    let out = Command::new(env!("CARGO_BIN_EXE_entrega")).arg("check").current_dir(r.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains(&format!("commit {absent} is not in the repository")), "{err}");
    assert!(err.contains("veredas-portal 2.0.0-alpha.3 is pinned by releases 2.0.0 and 2.0.1 at different code"), "{err}");
}

#[test]
fn an_executable_named_in_any_script_is_listed_and_a_glob_in_the_scratch_path_is_named() {
    let r = suite();
    r.write("crates/veredas-maquinar/src/bin/ação.rs", "fn main() {}\n");
    let c = r.commit("a bin named in Portuguese");
    let text = matching("2.0.0", &c).replace("bins = [\"veredas-maquinar\"]", "bins = [\"ação\", \"veredas-maquinar\"]");
    assert_eq!(checked(&r, &text), Vec::<String>::new());
    // The scratch folder lies in the tool's cache, whatever TMPDIR says; a parent with a glob
    // character is refused by name, never read by cargo as a pattern.
    let holder = tempfile::tempdir().unwrap();
    let globbed = holder.path().join("t[1]");
    std::fs::create_dir_all(&globbed).unwrap();
    let mut refusals = Vec::new();
    assert!(manifest::components_at_in(r.path(), &c, &globbed, &mut refusals).unwrap().is_empty());
    assert_eq!(refusals, vec![format!("the scratch folder's parent {} holds a glob character cargo would read as a pattern", globbed.display())]);
}

#[test]
fn a_commit_whose_tree_does_not_read_is_its_own_refusal_and_hides_none() {
    let r = suite();
    let c1 = r.commit("the suite");
    r.write("crates/veredas-portal/src/main.rs", "fn main() { println!(\"changed\"); }\n");
    let c2 = r.commit("portal changed, its version not bumped");
    let mut ws = WORKSPACE.as_bytes().to_vec();
    ws.extend_from_slice(b"# caf\xe9\n");
    std::fs::write(r.path().join("Cargo.toml"), ws).unwrap();
    let c3 = r.commit("a root manifest that is not UTF-8");
    for (rel, c) in [("2.0.0", c1.as_str()), ("2.0.1", c2.as_str()), ("2.0.2", c3.as_str())] {
        r.write(&format!("releases/{rel}.toml"), &matching(rel, c));
    }
    r.commit("three releases");
    let mut refusals = Vec::new();
    assert!(manifest::components_at(r.path(), &c3, &mut refusals).unwrap().is_empty());
    assert_eq!(refusals, vec![format!("commit {c3}: its Cargo.toml is not UTF-8")]);
    let out = Command::new(env!("CARGO_BIN_EXE_entrega")).arg("check").current_dir(r.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains(&format!("commit {c3}: its Cargo.toml is not UTF-8")), "{err}");
    assert!(err.contains("veredas-portal 2.0.0-alpha.3 is pinned by releases 2.0.0 and 2.0.1 at different code"), "{err}");
}

#[test]
fn a_relative_cache_holding_a_glob_character_is_judged_where_it_lies() {
    let holder = tempfile::tempdir().unwrap();
    let place = holder.path().join("t[1]");
    std::fs::create_dir_all(&place).unwrap();
    let r = suite();
    let c = r.commit("the suite");
    r.write("releases/2.0.0.toml", &matching("2.0.0", &c));
    r.commit("a release");
    // The repository cloned under `t[1]`, the cache named relative to it.
    let inside = place.join("repo");
    let clone = Command::new("git").args(["clone", "-q"]).arg(r.path()).arg(&inside).output().unwrap();
    assert!(clone.status.success());
    let out = Command::new(env!("CARGO_BIN_EXE_entrega")).arg("check").env("ENTREGA_CACHE", "relcache").current_dir(&inside).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8(out.stderr).unwrap();
    let parent = inside.join("relcache/tmp");
    assert!(err.contains(&format!("the scratch folder's parent {} holds a glob character cargo would read as a pattern", parent.display())), "{err}");
}

#[test]
fn an_annotated_tags_id_is_no_commit() {
    let r = suite();
    let c = r.commit("the suite");
    r.git(&["tag", "-a", "suite-2.0.0", "-m", "a tag"]);
    let tag = r.git(&["rev-parse", "suite-2.0.0"]);
    assert_ne!(tag, c);
    assert_eq!(checked(&r, &matching("2.0.0", &tag)), vec![format!("{tag} names a tag, not a commit: a release names the commit itself")]);
    assert_eq!(checked(&r, &matching("2.0.0", &c)), Vec::<String>::new());
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
    let (none, no_refusal) = manifest::load_dir(&dir);
    assert!(none.is_empty() && no_refusal.is_empty(), "no folder, no release");
    r.write("releases/2.0.0.toml", &matching("2.0.0", &c));
    let (loaded, refusals) = manifest::load_dir(&dir);
    assert_eq!((loaded.len(), refusals.len()), (1, 0));
    assert_eq!(loaded[0].1.release.to_string(), "2.0.0");
    r.write("releases/2.0.1.toml", &matching("2.0.2", &c));
    r.write("releases/notes.md", "not a manifest\n");
    let (loaded, refusals) = manifest::load_dir(&dir);
    let names: Vec<String> = loaded.iter().map(|(_, m)| m.release.to_string()).collect();
    assert_eq!(names, ["2.0.0", "2.0.2"], "what reads is still read beside what is refused");
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
    // A file that is no manifest beside one that does not check: both said, the exit 2.
    r.write("releases/notes.md", "not a manifest\n");
    let out = run();
    assert_eq!(out.status.code(), Some(2));
    let releases = r.path().canonicalize().unwrap().join("releases");
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        format!(
            "{}: a releases folder holds manifests (`<release>.toml`) alone\n{}: veredas-maquinar is 2.1.0 at {c}, not 2.1.1\n",
            releases.join("notes.md").display(),
            releases.join("2.0.0.toml").display()
        )
    );
    let usage = Command::new(env!("CARGO_BIN_EXE_entrega")).output().unwrap();
    assert_eq!(usage.status.code(), Some(1));
    assert_eq!(String::from_utf8(usage.stderr).unwrap(), "usage: entrega check [<manifest.toml>...]\n       entrega cut <release> [--trial] [--out <dir>]\n       entrega maquina <release>\n       entrega prova <release>\n");
}

#[test]
fn every_release_of_this_repository_checks_against_its_history() {
    let root = repo_root().canonicalize().unwrap();
    let (all, refusals) = manifest::load_dir(&root.join("releases"));
    assert_eq!(refusals, Vec::<String>::new());
    let names: Vec<String> = all.iter().map(|(_, m)| m.release.to_string()).collect();
    assert!(names.iter().any(|n| n == "2.0.0-alpha.1"), "the first release is in releases/: {names:?}");
    for (p, m) in &all {
        assert_eq!(manifest::check(&root, m).unwrap(), Vec::<String>::new(), "{}", p.display());
    }
    assert_eq!(manifest::check_together(&root, &all).unwrap(), Vec::<String>::new());
}
