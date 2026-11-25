//! C-290: the suite release's manifest. The plan's worked example read as its keys say; every
//! refusal of the reader by its message; a manifest checked against throwaway repositories'
//! history (a component left out, one too many, a version or an executable that differs, a commit
//! that is not there or not an ancestor, two releases pinning one version at different code, a
//! file named after another release); the executable on both outcomes; and every manifest of this
//! repository checked against its own history.

use std::path::{Path, PathBuf};
use std::process::Command;

use alforge::manifest::{self, Manifest};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The first `toml` block of the plan: the worked example.
fn example() -> String {
    let plan = std::fs::read_to_string(repo_root().join("docs/alforge-plan.md")).unwrap();
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
            .args(["-c", "user.name=alforge", "-c", "user.email=alforge@example.invalid", "-c", "commit.gpgsign=false"])
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
    // A bin with required features (here one the defaults turn on through another feature, and
    // one they leave off) is shipped as cargo's default build makes it, which the cut sees (C-291,
    // docs/questions.md Q443): a manifest may name it or leave it out, never name a bin there is not.
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
            want("veredas-core", &[]),
            want("veredas-maquinar", &["a"]),
            want("veredas-portal", &["porteiro"]),
            want("veredas-velho", &["velho"]),
        ]
    );
    assert_eq!(held["veredas-core"].optional.iter().collect::<Vec<_>>(), ["extra", "servido"]);
    assert!(held.values().filter(|h| h.name != "veredas-core").all(|h| h.optional.is_empty()));
    let text = |core_bins: &str| format!(
        "format = \"veredas-release\"\nversion = 1\nrelease = \"2.0.0\"\ncommit = \"{c}\"\ntarget = \"x86_64-unknown-linux-gnu\"\ntoolchain = \"rustc 1.100.0\"\n\n\
         [[component]]\nname = \"veredas-core\"\nversion = \"2.0.0\"\nbins = [{core_bins}]\n\n\
         [[component]]\nname = \"veredas-maquinar\"\nversion = \"2.0.0\"\nbins = [\"a\"]\n\n\
         [[component]]\nname = \"veredas-portal\"\nversion = \"2.0.0\"\nbins = [\"porteiro\"]\n\n\
         [[component]]\nname = \"veredas-velho\"\nversion = \"2.0.0\"\nbins = [\"velho\"]\n\n\
         [tested]\nsuite = \"cargo nextest run --workspace\"\ntogether = \"the bundle's bench and supervisão on its plant\"\n"
    );
    for shipped in ["", "\"servido\"", "\"extra\", \"servido\""] {
        assert_eq!(checked(&r, &text(shipped)), Vec::<String>::new(), "{shipped}");
    }
    assert_eq!(checked(&r, &text("\"servido\", \"outro\"")), [format!("veredas-core builds no `outro` at {c}")]);
    // A workspace cargo cannot read is refused with cargo's reason.
    r.write("crates/veredas-velho/Cargo.toml", "[package]\nname = 3\n");
    let c2 = r.commit("a manifest cargo refuses");
    let mut refusals = Vec::new();
    manifest::components_at(r.path(), &c2, &mut refusals).unwrap();
    assert_eq!(refusals.len(), 1);
    assert!(refusals[0].starts_with(&format!("commit {c2}: cargo cannot read its workspace: error")), "{refusals:?}");
    assert!(refusals[0].contains("invalid type: integer `3`, expected a string"), "cargo's reason: {refusals:?}");
    assert!(!refusals[0].contains("alforge-tree-"), "no scratch path in the reason: {refusals:?}");
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
    let n = alforge::git::export_tree(r.path(), &c2, &["crates/veredas-maquinar"], out.path()).unwrap();
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
    std::fs::write(parent.path().join("Cargo.toml"), "[workspace]\nmembers = [\"alforge-tree-*/tree\", \"crates/*\"]\n").unwrap();
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
        vec!["veredas-portal 2.0.0-alpha.3 is pinned by releases 2.0.0 and 2.0.1 at different build inputs (crates/veredas-portal/): a version names everything its build reads".to_string()]
    );
    // Through the executable, every refusal said: the absent commit's and the one tree's.
    for (rel, c) in [("2.0.0", c1.as_str()), ("2.0.1", c2.as_str()), ("2.0.2", absent)] {
        r.write(&format!("releases/{rel}.toml"), &matching(rel, c));
    }
    r.commit("three releases");
    let out = Command::new(env!("CARGO_BIN_EXE_alforge")).arg("check").current_dir(r.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8(out.stderr).unwrap();
    assert!(err.contains(&format!("commit {absent} is not in the repository")), "{err}");
    assert!(err.contains("veredas-portal 2.0.0-alpha.3 is pinned by releases 2.0.0 and 2.0.1 at different build inputs (crates/veredas-portal/)"), "{err}");
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
    let out = Command::new(env!("CARGO_BIN_EXE_alforge")).arg("check").current_dir(r.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let releases = r.path().canonicalize().unwrap().join("releases");
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        format!(
            "{}: commit {c3}: its Cargo.toml is not UTF-8\nveredas-portal 2.0.0-alpha.3 is pinned by releases 2.0.0 and 2.0.1 at different build inputs (crates/veredas-portal/): a version names everything its build reads\n",
            releases.join("2.0.2.toml").display()
        ),
        "said line for line, no false one"
    );
    // The unreadable release first: it charges no other release and hides nothing.
    for (rel, c) in [("2.0.0", c3.as_str()), ("2.0.1", c1.as_str()), ("2.0.2", c2.as_str())] {
        r.write(&format!("releases/{rel}.toml"), &matching(rel, c));
    }
    r.commit("the unreadable release first");
    let out = Command::new(env!("CARGO_BIN_EXE_alforge")).arg("check").current_dir(r.path()).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        format!(
            "{}: commit {c3}: its Cargo.toml is not UTF-8\nveredas-portal 2.0.0-alpha.3 is pinned by releases 2.0.1 and 2.0.2 at different build inputs (crates/veredas-portal/): a version names everything its build reads\n",
            releases.join("2.0.0.toml").display()
        )
    );
}

#[test]
fn a_cache_reached_through_a_link_and_a_dot_dot_reads_where_the_tree_was_written() {
    let r = suite();
    let c = r.commit("the suite");
    let d = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(d.path().join("far/away")).unwrap();
    std::fs::create_dir_all(d.path().join("home")).unwrap();
    std::os::unix::fs::symlink(d.path().join("far/away"), d.path().join("home/lnk")).unwrap();
    std::fs::create_dir_all(d.path().join("far/cache")).unwrap();
    let odd = d.path().join("home/lnk/../cache");
    let mut refusals = Vec::new();
    let held = manifest::components_at_in(r.path(), &c, &odd, &mut refusals).unwrap();
    assert_eq!((held.len(), refusals), (3, Vec::<String>::new()));
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
    let out = Command::new(env!("CARGO_BIN_EXE_alforge")).arg("check").env("ALFORGE_CACHE", "relcache").current_dir(&inside).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8(out.stderr).unwrap();
    let parent = inside.join("relcache/tmp");
    assert!(err.contains(&format!("the scratch folder's parent {} holds a glob character cargo would read as a pattern", parent.display())), "{err}");
}

#[test]
fn the_folder_is_judged_whole_and_a_shallow_clone_refused() {
    let r = suite();
    let c1 = r.commit("the suite");
    r.write("releases/2.0.0.toml", &matching("2.0.0", &c1));
    r.commit("release 2.0.0");
    r.write("docs/a.md", "a\n");
    r.commit("later");
    // `check` takes no file: a manifest is never judged apart from its folder.
    let named = Command::new(env!("CARGO_BIN_EXE_alforge")).args(["check", "releases/2.0.0.toml"]).current_dir(r.path()).output().unwrap();
    assert_eq!(named.status.code(), Some(1));
    let whole = Command::new(env!("CARGO_BIN_EXE_alforge")).arg("check").current_dir(r.path()).output().unwrap();
    assert_eq!(whole.status.code(), Some(0), "{}", String::from_utf8_lossy(&whole.stderr));
    // A shallow clone holding the release's commit: refused by name, never "not an ancestor".
    let shallow = tempfile::tempdir().unwrap();
    let clone = Command::new("git").args(["clone", "-q", "--depth", "1"]).arg(format!("file://{}", r.path().display())).arg(shallow.path()).output().unwrap();
    assert!(clone.status.success());
    let fetch = Command::new("git").arg("-C").arg(shallow.path()).args(["fetch", "-q", "--depth", "1", "origin", &c1]).output().unwrap();
    assert!(fetch.status.success(), "{}", String::from_utf8_lossy(&fetch.stderr));
    let m = manifest::load(&matching("2.0.0", &c1)).unwrap();
    assert_eq!(
        manifest::check(shallow.path(), &m).unwrap(),
        vec!["the repository is a shallow clone: its history is cut, and a release is checked against the whole of it".to_string()]
    );
}

#[test]
fn a_repository_at_an_odd_path_a_root_that_does_not_parse_and_braces_in_the_scratch_path() {
    // A repository whose path ends in a space: its releases are found and judged.
    let r = suite();
    let c = r.commit("the suite");
    r.write("releases/2.0.0.toml", &matching("2.0.0", &c).replace("version = \"2.1.0\"", "version = \"9.9.9\""));
    r.commit("a false release");
    let holder = tempfile::tempdir().unwrap();
    let odd = holder.path().join("b ");
    let clone = Command::new("git").args(["clone", "-q"]).arg(r.path()).arg(&odd).output().unwrap();
    assert!(clone.status.success());
    let out = Command::new(env!("CARGO_BIN_EXE_alforge")).arg("check").current_dir(&odd).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(out.stderr).unwrap(),
        format!("{}: veredas-maquinar is 2.1.0 at {c}, not 9.9.9\n", odd.canonicalize().unwrap().join("releases/2.0.0.toml").display())
    );
    // A root manifest that does not parse: cargo's reason, not "no workspace".
    let r = suite();
    r.write("Cargo.toml", "[workspace]\nmembers = [\"tools/ferramenta\", \"crates/*\"\n");
    let c = r.commit("a root manifest that does not parse");
    let mut refusals = Vec::new();
    assert!(manifest::components_at(r.path(), &c, &mut refusals).unwrap().is_empty());
    assert_eq!(refusals.len(), 1);
    assert!(refusals[0].starts_with(&format!("commit {c}: cargo cannot read its workspace: ")), "{refusals:?}");
    assert!(!refusals[0].contains("no workspace"), "{refusals:?}");
    // Braces and `]` in the scratch folder's path are no pattern to cargo: the members are found.
    let r = suite();
    let c = r.commit("the suite");
    for name in ["k{1}", "k]1", "k}1"] {
        let parent = holder.path().join(name);
        std::fs::create_dir_all(&parent).unwrap();
        let mut refusals = Vec::new();
        let held = manifest::components_at_in(r.path(), &c, &parent, &mut refusals).unwrap();
        assert_eq!((held.len(), refusals), (3, Vec::<String>::new()), "{name}");
    }
}

#[test]
fn a_submodule_a_sha256_repository_and_a_parent_that_is_not_utf8_are_refused_by_name() {
    // A gitlink under crates/: refused by its path, never left out without a word.
    let r = suite();
    let c = r.commit("the suite");
    r.git(&["update-index", "--add", "--cacheinfo", &format!("160000,{c},crates/veredas-x")]);
    r.git(&["commit", "-q", "-m", "a submodule"]);
    let with = r.git(&["rev-parse", "HEAD"]);
    let mut refusals = Vec::new();
    assert!(manifest::components_at(r.path(), &with, &mut refusals).unwrap().is_empty());
    assert_eq!(refusals, vec![format!("commit {with}: the submodule `crates/veredas-x`: a release's tree holds no submodule, whose code its history does not hold")]);
    // A repository of SHA-256 ids, a branch named like a SHA-1 id: refused by name.
    let d = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git").arg("-C").arg(d.path()).args(["-c", "user.name=e", "-c", "user.email=e@example.invalid"]).args(args).output().unwrap();
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    git(&["init", "-q", "--object-format=sha256", "-b", "main"]);
    std::fs::write(d.path().join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "a sha256 repository"]);
    let name = "0123456789abcdef0123456789abcdef01234567";
    git(&["branch", name]);
    let text = matching("2.0.0", name);
    assert_eq!(manifest::check(d.path(), &manifest::load(&text).unwrap()).unwrap(), vec!["the repository names its objects by sha256, not sha1: a release's commit is a SHA-1 id".to_string()]);
    // In a SHA-1 repository, a branch named like an id is no commit's own id.
    let r = suite();
    let c = r.commit("the suite");
    let fake = "0123456789abcdef0123456789abcdef01234567";
    r.git(&["branch", fake, &c]);
    assert!(alforge::git::is_commit_id(r.path(), &c).unwrap());
    assert!(!alforge::git::is_commit_id(r.path(), fake).unwrap());
    assert_eq!(checked(&r, &matching("2.0.0", &c)), Vec::<String>::new());
    // SHA-1 reads a 40-hex string as an id first: the branch is never taken for it.
    assert_eq!(checked(&r, &matching("2.0.0", fake)), vec![format!("commit {fake} is not in the repository")]);
    // A scratch parent that is not UTF-8: named, never handed to cargo.
    use std::os::unix::ffi::OsStrExt;
    let holder = tempfile::tempdir().unwrap();
    let parent = holder.path().join(std::ffi::OsStr::from_bytes(b"caf\xe9"));
    std::fs::create_dir_all(&parent).unwrap();
    let mut refusals = Vec::new();
    assert!(manifest::components_at_in(r.path(), &c, &parent, &mut refusals).unwrap().is_empty());
    assert_eq!(refusals.len(), 1);
    assert!(refusals[0].ends_with("is not UTF-8: cargo takes no such path"), "{refusals:?}");
}

#[test]
fn a_false_pin_charges_no_true_release_and_hides_no_conflict() {
    let run = |r: &Repo| {
        let out = Command::new(env!("CARGO_BIN_EXE_alforge")).arg("check").current_dir(r.path()).output().unwrap();
        (out.status.code(), String::from_utf8(out.stderr).unwrap())
    };
    // Case a: portal absent at the first release's commit, which lists it all the same. The
    // workspace's members by a glob, so its manifest is one at every commit and charges no one.
    let r = suite();
    r.write("Cargo.toml", "[workspace]\nmembers = [\"tools/ferramenta\", \"crates/*\"]\n");
    let portal: Vec<(String, Vec<u8>)> = ["Cargo.toml", "src/main.rs", "src/bin/porteiro.rs"]
        .iter()
        .map(|f| (format!("crates/veredas-portal/{f}"), std::fs::read(r.path().join("crates/veredas-portal").join(f)).unwrap()))
        .collect();
    std::fs::remove_dir_all(r.path().join("crates/veredas-portal")).unwrap();
    let c0 = r.commit("no portal yet");
    for (f, bytes) in &portal {
        r.write(f, std::str::from_utf8(bytes).unwrap());
    }
    let c1 = r.commit("portal joins");
    r.write("crates/veredas-portal/src/main.rs", "fn main() { println!(\"changed\"); }\n");
    let c2 = r.commit("portal changed, its version not bumped");
    for (rel, c) in [("2.0.0", c0.as_str()), ("2.0.1", c1.as_str()), ("2.0.2", c2.as_str())] {
        r.write(&format!("releases/{rel}.toml"), &matching(rel, c));
    }
    r.commit("three releases, the first false");
    let releases = r.path().canonicalize().unwrap().join("releases");
    assert_eq!(
        run(&r),
        (
            Some(2),
            format!(
                "{}: veredas-portal is no component at {c0}\nveredas-portal 2.0.0-alpha.3 is pinned by releases 2.0.1 and 2.0.2 at different build inputs (crates/veredas-portal/): a version names everything its build reads\n",
                releases.join("2.0.0.toml").display()
            )
        )
    );
    // Case b: the first release states a version its commit does not hold; two true releases at
    // one commit after it are charged with nothing.
    let r = suite();
    r.write("crates/veredas-portal/Cargo.toml", "[package]\nname = \"veredas-portal\"\nversion = \"2.0.0-alpha.2\"\nedition = \"2024\"\n\n[[bin]]\nname = \"veredas-portal\"\npath = \"src/main.rs\"\n");
    let c0 = r.commit("portal at alpha.2");
    r.write("crates/veredas-portal/Cargo.toml", "[package]\nname = \"veredas-portal\"\nversion = \"2.0.0-alpha.3\"\nedition = \"2024\"\n\n[[bin]]\nname = \"veredas-portal\"\npath = \"src/main.rs\"\n");
    let c1 = r.commit("portal at alpha.3");
    for (rel, c) in [("2.0.0", c0.as_str()), ("2.0.1", c1.as_str()), ("2.0.2", c1.as_str())] {
        r.write(&format!("releases/{rel}.toml"), &matching(rel, c));
    }
    r.commit("three releases, the first false");
    let releases = r.path().canonicalize().unwrap().join("releases");
    assert_eq!(
        run(&r),
        (Some(2), format!("{}: veredas-portal is 2.0.0-alpha.2 at {c0}, not 2.0.0-alpha.3\n", releases.join("2.0.0.toml").display()))
    );
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
        vec!["veredas-portal 2.0.0-alpha.3 is pinned by releases 2.0.0 and 2.0.2 at different build inputs (crates/veredas-portal/): a version names everything its build reads".to_string()]
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
    let run = || Command::new(env!("CARGO_BIN_EXE_alforge")).arg("check").current_dir(r.path().join("crates")).output().unwrap();
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
    let usage = Command::new(env!("CARGO_BIN_EXE_alforge")).output().unwrap();
    assert_eq!(usage.status.code(), Some(1));
    assert_eq!(String::from_utf8(usage.stderr).unwrap(), "usage: alforge check\n       alforge cut <release> [--trial] [--out <dir>]\n       alforge maquina <release>\n       alforge prova <release>\n");
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

const SERDE: &str = "[[package]]\nname = \"serde\"\nversion = \"1.0.200\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"aaaa\"\n";
const TEMPFILE: &str = "[[package]]\nname = \"tempfile\"\nversion = \"3.10.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"bbbb\"\n";
const ITOA: &str = "[[package]]\nname = \"itoa\"\nversion = \"1.0.11\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"cccc\"\n";

/// The lock file of [`wider`], with `serde`, `tempfile` and `itoa` as given.
fn lock(serde: &str, tempfile: &str, itoa: &str) -> String {
    format!(
        "version = 4\n\n[[package]]\nname = \"garfo\"\nversion = \"0.3.0\"\ndependencies = [\n \"itoa\",\n]\n\n{itoa}\n{serde}\n{tempfile}\n\
         [[package]]\nname = \"veredas-core\"\nversion = \"2.0.0\"\n\n\
         [[package]]\nname = \"veredas-maquinar\"\nversion = \"2.1.0\"\ndependencies = [\n \"garfo\",\n]\n\n\
         [[package]]\nname = \"veredas-portal\"\nversion = \"2.0.0-alpha.3\"\ndependencies = [\n \"serde\",\n \"tempfile\",\n \"veredas-core\",\n]\n"
    )
}

/// A suite whose portal uses core by path, `serde` for what it ships and `tempfile` for its tests
/// alone, both pinned by the lock file, and names two files outside its folder (a module by
/// `#[path]`, a text by `include_str!`); whose bench uses `garfo` by path, a fork kept in the tree
/// in a workspace of its own, excluded from the suite's, that uses `itoa`.
fn wider() -> Repo {
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nmembers = [\"crates/veredas-core\", \"crates/veredas-maquinar\", \"crates/veredas-portal\"]\nexclude = [\"forks/garfo\"]\n");
    r.write("Cargo.lock", &lock(SERDE, TEMPFILE, ITOA));
    r.write("crates/veredas-core/Cargo.toml", "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\nedition = \"2024\"\n");
    r.write("crates/veredas-core/src/lib.rs", "\n");
    r.write(
        "crates/veredas-portal/Cargo.toml",
        "[package]\nname = \"veredas-portal\"\nversion = \"2.0.0-alpha.3\"\nedition = \"2024\"\n\n[[bin]]\nname = \"veredas-portal\"\npath = \"src/main.rs\"\n\n\
         [dependencies]\nveredas-core = { path = \"../veredas-core\" }\nserde = \"1\"\n\n[dev-dependencies]\ntempfile = \"3\"\n",
    );
    r.write("crates/veredas-portal/src/main.rs", "#[path = \"../../../docs/shared.rs\"]\nmod shared;\n\nfn main() {\n    let _ = include_str!(\"../../../docs/notes.txt\");\n}\n");
    r.write("crates/veredas-portal/src/bin/porteiro.rs", "fn main() {}\n");
    r.write("crates/veredas-portal/tests/reads.rs", "const DOC: &str = include_str!(\"../../../docs/test-only.md\");\n");
    r.write("docs/shared.rs", "pub fn shared() {}\n");
    r.write("docs/notes.txt", "notes\n");
    r.write("docs/test-only.md", "read by a test alone\n");
    r.write("docs/other.md", "named by nothing\n");
    r.write("crates/veredas-maquinar/Cargo.toml", "[package]\nname = \"veredas-maquinar\"\nversion = \"2.1.0\"\nedition = \"2024\"\n\n[dependencies]\ngarfo = { path = \"../../forks/garfo/lib\" }\n");
    r.write("crates/veredas-maquinar/src/main.rs", "fn main() {}\n");
    r.write("forks/garfo/Cargo.toml", "[workspace]\nmembers = [\"lib\"]\n\n[workspace.package]\nversion = \"0.3.0\"\n");
    r.write("forks/garfo/lib/Cargo.toml", "[package]\nname = \"garfo\"\nversion.workspace = true\nedition = \"2024\"\n\n[dependencies]\nitoa = \"1\"\n");
    r.write("forks/garfo/lib/src/lib.rs", "\n");
    r
}

/// The manifest that pins [`wider`]'s three components at `commit`.
fn pins(release: &str, commit: &str) -> (PathBuf, Manifest) {
    let text = format!(
        "format = \"veredas-release\"\nversion = 1\nrelease = \"{release}\"\ncommit = \"{commit}\"\ntarget = \"x86_64-unknown-linux-gnu\"\ntoolchain = \"rustc 1.100.0\"\n\n\
         [[component]]\nname = \"veredas-core\"\nversion = \"2.0.0\"\n\n\
         [[component]]\nname = \"veredas-maquinar\"\nversion = \"2.1.0\"\nbins = [\"veredas-maquinar\"]\n\n\
         [[component]]\nname = \"veredas-portal\"\nversion = \"2.0.0-alpha.3\"\nbins = [\"porteiro\", \"veredas-portal\"]\n\n\
         [tested]\nsuite = \"cargo nextest run --workspace\"\ntogether = \"the bundle's bench and supervisão on its plant\"\n"
    );
    (PathBuf::from(format!("{release}.toml")), manifest::load(&text).unwrap())
}

/// `edit` made on top of `base` and committed, the tree then put back at `base`: the commit.
fn changed(r: &Repo, base: &str, msg: &str, edit: impl FnOnce(&Repo)) -> String {
    edit(r);
    let c = r.commit(msg);
    r.git(&["reset", "-q", "--hard", base]);
    c
}

/// What `check_together` says of releases 2.0.0 at `a` and 2.0.1 at `b`.
fn together(r: &Repo, a: &str, b: &str) -> Vec<String> {
    manifest::check_together(r.path(), &[pins("2.0.0", a), pins("2.0.1", b)]).unwrap()
}

fn charged(component: &str, version: &str, inputs: &str) -> String {
    format!("{component} {version} is pinned by releases 2.0.0 and 2.0.1 at different build inputs ({inputs}): a version names everything its build reads")
}

#[test]
fn a_version_names_everything_its_build_reads_but_the_toolchain() {
    let r = wider();
    let c1 = r.commit("the suite");
    let held = manifest::components_at(r.path(), &c1, &mut Vec::new()).unwrap();
    let names = |n: &str| -> Vec<String> { held[n].inputs.as_ref().unwrap().0.keys().cloned().collect() };
    let root = [".cargo/config", ".cargo/config.toml", "Cargo.toml"];
    let with = |more: &[&str]| -> Vec<String> {
        let mut v: Vec<String> = root.iter().chain(more).map(|s| s.to_string()).collect();
        v.sort();
        v
    };
    // Portal: its folder and core's, the files it names (its test's too: the folder is read whole,
    // docs/questions.md Q445), serde; never tempfile, its tests' dependency alone.
    assert_eq!(names("veredas-portal"), with(&["Cargo.lock: serde 1.0.200", "crates/veredas-core/", "crates/veredas-portal/", "docs/notes.txt", "docs/shared.rs", "docs/test-only.md"]));
    assert_eq!(held["veredas-portal"].inputs.as_ref().unwrap().0["Cargo.lock: serde 1.0.200"], "aaaa");
    assert_eq!(held["veredas-portal"].inputs.as_ref().unwrap().0[".cargo/config.toml"], "absent");
    // The bench: the fork's folder and its workspace's manifest, read by cargo there, and what the
    // fork uses.
    assert_eq!(names("veredas-maquinar"), with(&["Cargo.lock: itoa 1.0.11", "crates/veredas-maquinar/", "forks/garfo/Cargo.toml", "forks/garfo/lib/"]));
    assert_eq!(names("veredas-core"), with(&["crates/veredas-core/"]));
    assert_eq!(together(&r, &c1, &c1), Vec::<String>::new());

    // Each input changed alone, from the same commit: who is charged, by which input.
    type Case = (&'static str, Box<dyn FnOnce(&Repo)>, Vec<String>);
    let cases: Vec<Case> = vec![
        ("a file nothing names", Box::new(|r: &Repo| r.write("docs/other.md", "changed\n")), vec![]),
        ("a file portal's test alone names (Q445)", Box::new(|r: &Repo| r.write("docs/test-only.md", "changed\n")), vec![charged("veredas-portal", "2.0.0-alpha.3", "docs/test-only.md")]),
        (
            "portal's test",
            Box::new(|r: &Repo| r.write("crates/veredas-portal/tests/reads.rs", "const DOC: &str = include_str!(\"../../../docs/test-only.md\");\nconst MORE: u8 = 1;\n")),
            vec![charged("veredas-portal", "2.0.0-alpha.3", "crates/veredas-portal/")],
        ),
        (
            "core's code",
            Box::new(|r: &Repo| r.write("crates/veredas-core/src/lib.rs", "pub fn changed() {}\n")),
            vec![charged("veredas-core", "2.0.0", "crates/veredas-core/"), charged("veredas-portal", "2.0.0-alpha.3", "crates/veredas-core/")],
        ),
        ("the module portal names by #[path]", Box::new(|r: &Repo| r.write("docs/shared.rs", "pub fn changed() {}\n")), vec![charged("veredas-portal", "2.0.0-alpha.3", "docs/shared.rs")]),
        ("the text portal includes", Box::new(|r: &Repo| r.write("docs/notes.txt", "changed\n")), vec![charged("veredas-portal", "2.0.0-alpha.3", "docs/notes.txt")]),
        (
            "serde updated in the lock file",
            Box::new(|r: &Repo| r.write("Cargo.lock", &lock(&SERDE.replace("1.0.200", "1.0.201").replace("aaaa", "abab"), TEMPFILE, ITOA))),
            vec![charged("veredas-portal", "2.0.0-alpha.3", "Cargo.lock: serde 1.0.200, Cargo.lock: serde 1.0.201")],
        ),
        ("tempfile, portal's tests' alone, updated", Box::new(|r: &Repo| r.write("Cargo.lock", &lock(SERDE, &TEMPFILE.replace("3.10.0", "3.11.0"), ITOA))), vec![]),
        (
            "itoa's checksum, under the fork",
            Box::new(|r: &Repo| r.write("Cargo.lock", &lock(SERDE, TEMPFILE, &ITOA.replace("cccc", "cdcd")))),
            vec![charged("veredas-maquinar", "2.1.0", "Cargo.lock: itoa 1.0.11")],
        ),
        ("the fork's code", Box::new(|r: &Repo| r.write("forks/garfo/lib/src/lib.rs", "pub fn changed() {}\n")), vec![charged("veredas-maquinar", "2.1.0", "forks/garfo/lib/")]),
        (
            "the fork's workspace manifest",
            Box::new(|r: &Repo| r.write("forks/garfo/Cargo.toml", "[workspace]\nmembers = [\"lib\"]\n\n[workspace.package]\nversion = \"0.3.0\"\nedition = \"2024\"\n")),
            vec![charged("veredas-maquinar", "2.1.0", "forks/garfo/Cargo.toml")],
        ),
        (
            "the workspace's manifest",
            Box::new(|r: &Repo| {
                let t = std::fs::read_to_string(r.path().join("Cargo.toml")).unwrap();
                r.write("Cargo.toml", &format!("{t}\n[profile.release]\nlto = true\n"));
            }),
            vec![charged("veredas-core", "2.0.0", "Cargo.toml"), charged("veredas-maquinar", "2.1.0", "Cargo.toml"), charged("veredas-portal", "2.0.0-alpha.3", "Cargo.toml")],
        ),
    ];
    for (what, edit, want) in cases {
        let c = changed(&r, &c1, what, edit);
        assert_eq!(together(&r, &c1, &c), want, "{what}");
    }
}

#[test]
fn build_inputs_that_do_not_read_are_refused_where_a_version_is_pinned_twice() {
    let r = wider();
    let c1 = r.commit("the suite");
    let unread = |component: &str, why: &str| format!("{component} is pinned by releases 2.0.0 and 2.0.1, and its build inputs at 2.0.1's commit do not read: {why}");
    // No lock file: what depends on a crate it would pin is refused; core, which depends on none,
    // is not.
    let c = changed(&r, &c1, "no lock file", |r| std::fs::remove_file(r.path().join("Cargo.lock")).unwrap());
    assert_eq!(
        together(&r, &c1, &c),
        [
            unread("veredas-maquinar 2.1.0", "garfo depends on `itoa` and the commit holds no Cargo.lock to pin it"),
            unread("veredas-portal 2.0.0-alpha.3", "veredas-portal depends on `serde` and the commit holds no Cargo.lock to pin it"),
        ]
    );
    // A lock file that is not the workspace's: portal not in it.
    let c = changed(&r, &c1, "a lock without portal", |r| {
        let l = lock(SERDE, TEMPFILE, ITOA);
        r.write("Cargo.lock", &l[..l.find("[[package]]\nname = \"veredas-portal\"").unwrap()]);
    });
    assert_eq!(together(&r, &c1, &c), [unread("veredas-portal 2.0.0-alpha.3", "its Cargo.lock does not pin veredas-portal 2.0.0-alpha.3: the lock file is not this workspace's")]);
    // A dependency string the lock answers with no package, and one it answers twice.
    let c = changed(&r, &c1, "a lock naming serde 2", |r| r.write("Cargo.lock", &lock(SERDE, TEMPFILE, ITOA).replace(" \"serde\",", " \"serde 2.0.0\",")));
    assert_eq!(together(&r, &c1, &c), [unread("veredas-portal 2.0.0-alpha.3", "its Cargo.lock names `serde 2.0.0` and pins no such package")]);
    let c = changed(&r, &c1, "two serdes", |r| r.write("Cargo.lock", &lock(&format!("{SERDE}\n{}", SERDE.replace("1.0.200", "1.0.201")), TEMPFILE, ITOA)));
    assert_eq!(together(&r, &c1, &c), [unread("veredas-portal 2.0.0-alpha.3", "its Cargo.lock names `serde`, which more than one of its packages answers")]);
    // A path a source names that climbs out of the tree.
    let c = changed(&r, &c1, "a module outside the tree", |r| r.write("crates/veredas-portal/src/bin/porteiro.rs", "#[path = \"../../../../../fora.rs\"]\nmod fora;\nfn main() {}\n"));
    assert_eq!(
        together(&r, &c1, &c),
        [unread("veredas-portal 2.0.0-alpha.3", "crates/veredas-portal/src/bin/porteiro.rs names `../../../../../fora.rs`, outside the commit's tree")]
    );
    // A target starting from a file out of the tree.
    let c = changed(&r, &c1, "a library out of the tree", |r| {
        let t = std::fs::read_to_string(r.path().join("crates/veredas-core/Cargo.toml")).unwrap();
        r.write("crates/veredas-core/Cargo.toml", &format!("{t}\n[lib]\npath = \"../../../../fora.rs\"\n"));
    });
    let got = together(&r, &c1, &c);
    assert!(
        got.iter().any(|g| g.starts_with(&unread("veredas-core 2.0.0", "veredas-core builds from `")) && g.ends_with("fora.rs`, outside the commit's tree")),
        "{got:?}"
    );
    // A fork's dependency by a path out of the tree, to a package cargo reads there: refused as the
    // commit's, as the workspace's own are (one cargo cannot read is refused with cargo's reason).
    let outside = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(outside.path().join("src")).unwrap();
    std::fs::write(outside.path().join("Cargo.toml"), "[package]\nname = \"fora\"\nversion = \"0.1.0\"\nedition = \"2024\"\n").unwrap();
    std::fs::write(outside.path().join("src/lib.rs"), "\n").unwrap();
    let c = changed(&r, &c1, "a fork reaching out", |r| {
        let t = std::fs::read_to_string(r.path().join("forks/garfo/lib/Cargo.toml")).unwrap();
        r.write("forks/garfo/lib/Cargo.toml", &format!("{t}fora = {{ path = \"{}\" }}\n", outside.path().display()));
    });
    let mut refusals = Vec::new();
    manifest::components_at(r.path(), &c, &mut refusals).unwrap();
    assert_eq!(refusals, [format!("commit {c}: the package `garfo` depends on `fora` by a path outside the commit's tree")]);
    // A fork cargo cannot read: its users' refusal, with cargo's reason.
    let c = changed(&r, &c1, "a fork that does not parse", |r| r.write("forks/garfo/lib/Cargo.toml", "[package\n"));
    let got = together(&r, &c1, &c);
    assert_eq!(got.len(), 1, "{got:?}");
    assert!(got[0].starts_with(&unread("veredas-maquinar 2.1.0", "cargo cannot read the package at forks/garfo/lib: ")), "{got:?}");
    // At the first release's commit too, named as that release's.
    let got = manifest::check_together(r.path(), &[pins("2.0.0", &c), pins("2.0.1", &c1)]).unwrap();
    assert!(got[0].starts_with("veredas-maquinar 2.1.0 is pinned by releases 2.0.0 and 2.0.1, and its build inputs at 2.0.0's commit do not read: cargo cannot read the package at forks/garfo/lib: "), "{got:?}");
}

#[test]
fn a_source_names_files_by_literal_paths_alone() {
    // Read from rustc's tokens: what lexes as one literal is one, whatever quotes a char literal or
    // a raw string holds, and comments between tokens are no tokens.
    let src = "#[path = \"a.rs\"]\nmod a;\n#[path=\"b/c.rs\"] mod c;\nconst Q: char = '\"';\nconst R: &str = r#\"say \"hi\"#;\nconst W: &str = r\"C:\\\";\n\
        const C: &str = include_str!(\"c.txt\");\nconst D: &[u8] = include_bytes! ( \"d.bin\" );\ninclude!(r\"e.rs\");\ninclude!(r#\"f \"q\".rs\"#);\n\
        const G: &str = include_str![\"g.txt\"];\nconst H: &str = include_str!(concat!(env!(\"OUT_DIR\"), \"/h.rs\"));\nconst I: &str = include_str!(\"i\\\\j \\\"k\\\".txt\");\n\
        // include_str!(\"in-a-comment.txt\")\n#[cfg_attr(unix, path = \"unix.rs\")]\nmod plat;\n#[cfg_attr(all(windows, feature = \"x)path\"), path = \"win.rs\", allow(dead_code))]\nmod win;\n\
        #[cfg_attr(a, cfg_attr(b, path = \"nested-cfg.rs\"))]\nmod n;\n# [ path = \"spaced.rs\" ]\nmod spaced;\nconst J: &str = include_str!(\"esc\\x2Fape\\u{2E}t\\txt\");\n\
        const K: &str = include_str!(\"line\\\r\n    two.txt\");\n#[path = /* c /* nested */ */ \"commented.rs\"] mod m;\nconst N: &str = include_str ! // a line comment\n    (\"bang-apart.txt\");\n\
        #[doc = include_str!(\"doc.md\")]\nmod documented {}\nconst P: &str = my_include_str!(\"not-rusts.txt\");\n";
    assert_eq!(
        alforge::inputs::literals(src),
        [
            "a.rs", "b/c.rs", "c.txt", "d.bin", "e.rs", "f \"q\".rs", "g.txt", "i\\j \"k\".txt", "unix.rs", "win.rs", "nested-cfg.rs", "spaced.rs", "esc/ape.t\txt", "linetwo.txt",
            "commented.rs", "bang-apart.txt", "doc.md",
        ],
        "a comment, `concat!` and an identifier ending in `include_str` name nothing"
    );
    // Which of them rustc reads as Rust, and so are read in turn: modules and `include!`.
    let rust: Vec<String> = alforge::inputs::named(src).into_iter().filter(|(_, r)| *r).map(|(p, _)| p).collect();
    assert_eq!(rust, ["a.rs", "b/c.rs", "e.rs", "f \"q\".rs", "unix.rs", "win.rs", "nested-cfg.rs", "spaced.rs", "commented.rs"]);
    // A text that does not lex as Rust is read for the same forms as text.
    assert_eq!(alforge::inputs::literals("include_str!(\"x.txt\") ) unbalanced"), ["x.txt"]);
}

#[test]
fn a_bin_with_required_features_ships_as_cargos_own_build_makes_it() {
    // Required features of every form cargo reads (an optional dependency's implicit feature
    // through `x/f`, a dependency's feature asked by its declaration, one never asked, a weak `?/`
    // on a dependency nothing enables, a feature that is off): alforge models none of it. The
    // cut's manifest names those cargo's default build made, as `cut::as_built` reads its output.
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\nresolver = \"3\"\n");
    r.write(".gitignore", "/target/\n");
    for lib in ["veredas-base", "veredas-cx", "veredas-dx"] {
        r.write(&format!("crates/{lib}/Cargo.toml"), &format!("[package]\nname = \"{lib}\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[features]\ndefault = [\"y\"]\nx = []\ny = []\n"));
        r.write(&format!("crates/{lib}/src/lib.rs"), "\n");
    }
    let bins = [("needs-b", "veredas-base"), ("needs-cx", "veredas-cx/x"), ("needs-cy", "veredas-cx/y"), ("needs-dx", "veredas-dx/y"), ("needs-z", "z")];
    let targets: String = bins.iter().map(|(b, f)| format!("\n[[bin]]\nname = \"{b}\"\npath = \"src/{b}.rs\"\nrequired-features = [\"{f}\"]\n")).collect();
    r.write(
        "crates/veredas-core/Cargo.toml",
        &format!(
            "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[features]\ndefault = [\"veredas-base/x\", \"fraco\"]\nfraco = [\"veredas-dx?/y\"]\nz = [\"dep:veredas-dx\"]\n\n\
             [dependencies]\nveredas-base = {{ path = \"../veredas-base\", optional = true }}\nveredas-cx = {{ path = \"../veredas-cx\", features = [\"x\"] }}\nveredas-dx = {{ path = \"../veredas-dx\", optional = true }}\n{targets}\n\
             [[bin]]\nname = \"sempre\"\npath = \"src/sempre.rs\"\n"
        ),
    );
    r.write("crates/veredas-core/src/lib.rs", "\n");
    for (b, _) in bins.iter().chain(&[("sempre", "")]) {
        r.write(&format!("crates/veredas-core/src/{b}.rs"), "fn main() {}\n");
    }
    let c = r.commit("required features of every form");
    let mut refusals = Vec::new();
    let held = manifest::components_at(r.path(), &c, &mut refusals).unwrap();
    assert_eq!(refusals, Vec::<String>::new());
    let core = &held["veredas-core"];
    let names = |s: &std::collections::BTreeSet<String>| s.iter().cloned().collect::<Vec<_>>();
    assert_eq!(names(&core.bins), ["sempre"]);
    assert_eq!(names(&core.optional), ["needs-b", "needs-cx", "needs-cy", "needs-dx", "needs-z"]);
    // Cargo's own build with the default features, alone.
    let target = tempfile::tempdir().unwrap();
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "--offline", "-q", "-p", "veredas-core", "--bins"])
        .env("CARGO_TARGET_DIR", target.path())
        .current_dir(r.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let built: Vec<String> = bins.iter().map(|(b, _)| b.to_string()).filter(|b| target.path().join("debug").join(b).exists()).collect();
    assert_eq!(built, ["needs-b", "needs-cx", "needs-cy"], "cargo's own reading: the dependency's default turns `y` on");
    let release = semver::Version::parse("9.9.9").unwrap();
    let (shipped, text, m, said) = alforge::cut::as_built(r.path(), &release, &c, "x86_64-unknown-linux-gnu", "rustc 1.100.0", alforge::cut::NOT_RUN, &held, &target.path().join("debug")).unwrap();
    let mut want = built.clone();
    want.push("sempre".into());
    assert_eq!(names(&shipped["veredas-core"].bins), want);
    assert!(shipped.values().all(|h| h.optional.is_empty()));
    let listed = &m.components.iter().find(|x| x.name == "veredas-core").unwrap().bins;
    assert_eq!(listed, &want);
    assert_eq!(manifest::load(&text).unwrap(), m);
    assert_eq!(said.len(), 2, "{said:?}");
}

/// A suite for what [`wider`] leaves out: portal uses core, which uses `veredas-base`, by path;
/// portal builds with `pkgbuild` (a build dependency), on unix with `libc`, `json` (serde_json under
/// another name) and `itoa` from crates.io beside a git `itoa` of the same version; core's library
/// starts from a file outside its folder; portal names a module by `cfg_attr`, a module under
/// `src/examples/`, and an outside module that names a file in turn; the bench uses `um`, a fork in
/// a workspace of its own that uses `dois`, in a third.
fn deeper() -> Repo {
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nmembers = [\"crates/veredas-base\", \"crates/veredas-core\", \"crates/veredas-maquinar\", \"crates/veredas-portal\"]\nexclude = [\"forks\"]\n");
    let reg = "registry+https://github.com/rust-lang/crates.io-index";
    let git = "git+https://example.invalid/itoa#0123456789abcdef0123456789abcdef01234567";
    r.write(
        "Cargo.lock",
        &format!(
            "version = 4\n\n[[package]]\nname = \"dois\"\nversion = \"0.1.0\"\n\n[[package]]\nname = \"itoa\"\nversion = \"1.0.11\"\nsource = \"{reg}\"\nchecksum = \"1111\"\n\n\
             [[package]]\nname = \"itoa\"\nversion = \"1.0.11\"\nsource = \"{git}\"\n\n[[package]]\nname = \"libc\"\nversion = \"0.2.170\"\nsource = \"{reg}\"\nchecksum = \"2222\"\n\n\
             [[package]]\nname = \"pkgbuild\"\nversion = \"1.0.0\"\nsource = \"{reg}\"\nchecksum = \"3333\"\n\n[[package]]\nname = \"serde_json\"\nversion = \"1.0.140\"\nsource = \"{reg}\"\nchecksum = \"4444\"\n\n\
             [[package]]\nname = \"um\"\nversion = \"0.1.0\"\ndependencies = [\n \"dois\",\n]\n\n[[package]]\nname = \"veredas-base\"\nversion = \"2.0.0\"\n\n\
             [[package]]\nname = \"veredas-core\"\nversion = \"2.0.0\"\ndependencies = [\n \"veredas-base\",\n]\n\n\
             [[package]]\nname = \"veredas-maquinar\"\nversion = \"2.1.0\"\ndependencies = [\n \"um\",\n]\n\n\
             [[package]]\nname = \"veredas-portal\"\nversion = \"2.0.0-alpha.3\"\ndependencies = [\n \"itoa 1.0.11 ({reg})\",\n \"libc\",\n \"pkgbuild\",\n \"serde_json\",\n \"veredas-core\",\n]\n"
        ),
    );
    r.write("crates/veredas-base/Cargo.toml", "[package]\nname = \"veredas-base\"\nversion = \"2.0.0\"\nedition = \"2024\"\nbuild = \"../../docs/base-build.rs\"\n");
    r.write("crates/veredas-base/src/lib.rs", "\n");
    r.write("docs/base-build.rs", "fn main() {}\n");
    r.write("crates/veredas-core/Cargo.toml", "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[lib]\npath = \"../../docs/core-lib.rs\"\n\n[dependencies]\nveredas-base = { path = \"../veredas-base\" }\n");
    r.write("docs/core-lib.rs", "pub fn core() {}\n");
    r.write(
        "crates/veredas-portal/Cargo.toml",
        "[package]\nname = \"veredas-portal\"\nversion = \"2.0.0-alpha.3\"\nedition = \"2024\"\n\n[[bin]]\nname = \"veredas-ponte\"\npath = \"../../docs/ponte.rs\"\n\n[dependencies]\nveredas-core = { path = \"../veredas-core\" }\njson = { package = \"serde_json\", version = \"1\" }\nitoa = \"1\"\n\n\
         [build-dependencies]\npkgbuild = \"1\"\n\n[target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n",
    );
    r.write(
        "crates/veredas-portal/src/main.rs",
        "#[cfg_attr(unix, path = \"../../../docs/unix.rs\")]\nmod plat;\n#[path = \"../../../docs/outer.rs\"]\nmod outer;\nmod examples;\n#[path = \"../examples/shared.rs\"]\nmod shared;\ninclude!(\"../../../docs/frag.in\");\nfn main() {}\n",
    );
    r.write("crates/veredas-portal/examples/shared.rs", "pub const S: &str = include_str!(\"../../../docs/shared.txt\");\n");
    r.write("docs/frag.in", "const F: &str = include_str!(\"frag.txt\");\n");
    r.write("crates/veredas-portal/src/examples/mod.rs", "pub const E: &str = include_str!(\"../../../../docs/example.txt\");\n");
    for (f, t) in [("docs/ponte.rs", "const P: &str = include_str!(\"ponte.txt\");\nfn main() {}\n"), ("docs/ponte.txt", "ponte\n"), ("docs/shared.txt", "shared\n"), ("docs/frag.txt", "frag\n"), ("docs/helper.txt", "helper\n"), ("docs/dois.txt", "dois\n"), ("docs/bench.txt", "bench\n"), ("docs/unix.rs", "\n"), ("docs/outer.rs", "pub const D: &str = include_str!(\"deep.txt\");\n"), ("docs/deep.txt", "deep\n"), ("docs/example.txt", "example\n")] {
        r.write(f, t);
    }
    // The bench's executable starts from its folder's `examples/`, which the sweep leaves out.
    r.write(
        "crates/veredas-maquinar/Cargo.toml",
        "[package]\nname = \"veredas-maquinar\"\nversion = \"2.1.0\"\nedition = \"2024\"\nautoexamples = false\n\n[[bin]]\nname = \"veredas-maquinar\"\npath = \"examples/bench-main.rs\"\n\n[dependencies]\num = { path = \"../../forks/um/lib\" }\n",
    );
    r.write("crates/veredas-maquinar/examples/bench-main.rs", "const B: &str = include_str!(\"../../../docs/bench.txt\");\nmod helper;\nfn main() {}\n");
    r.write("crates/veredas-maquinar/examples/helper.rs", "pub const H: &str = include_str!(\"../../../docs/helper.txt\");\n");
    r.write("forks/um/Cargo.toml", "[workspace]\nmembers = [\"lib\"]\n");
    r.write("forks/um/lib/Cargo.toml", "[package]\nname = \"um\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\ndois = { path = \"../../dois/lib\" }\n");
    r.write("forks/um/lib/src/lib.rs", "\n");
    r.write("forks/dois/Cargo.toml", "[workspace]\nmembers = [\"lib\"]\n");
    r.write("forks/dois/lib/Cargo.toml", "[package]\nname = \"dois\"\nversion = \"0.1.0\"\nedition = \"2024\"\n");
    r.write("forks/dois/lib/src/lib.rs", "pub const D: &str = include_str!(\"../../../../docs/dois.txt\");\n");
    r
}

#[test]
fn every_road_into_a_build_charges_the_components_it_reaches() {
    let r = deeper();
    let c1 = r.commit("the deeper suite");
    let held = manifest::components_at(r.path(), &c1, &mut Vec::new()).unwrap();
    let pins = |release: &str, commit: &str| -> (PathBuf, Manifest) {
        let text = format!(
            "format = \"veredas-release\"\nversion = 1\nrelease = \"{release}\"\ncommit = \"{commit}\"\ntarget = \"x86_64-unknown-linux-gnu\"\ntoolchain = \"rustc 1.100.0\"\n\n\
             [[component]]\nname = \"veredas-base\"\nversion = \"2.0.0\"\n\n[[component]]\nname = \"veredas-core\"\nversion = \"2.0.0\"\n\n\
             [[component]]\nname = \"veredas-maquinar\"\nversion = \"2.1.0\"\nbins = [\"veredas-maquinar\"]\n\n\
             [[component]]\nname = \"veredas-portal\"\nversion = \"2.0.0-alpha.3\"\nbins = [\"veredas-ponte\", \"veredas-portal\"]\n\n\
             [tested]\nsuite = \"cargo nextest run --workspace\"\ntogether = \"the bundle's bench and supervisão on its plant\"\n"
        );
        (PathBuf::from(format!("{release}.toml")), manifest::load(&text).unwrap())
    };
    assert_eq!(manifest::check(r.path(), &pins("2.0.0", &c1).1).unwrap(), Vec::<String>::new());
    let keys = |n: &str| -> Vec<String> { held[n].inputs.as_ref().unwrap().0.keys().filter(|k| !k.starts_with(".cargo") && *k != "Cargo.toml").cloned().collect() };
    assert_eq!(
        keys("veredas-portal"),
        [
            "Cargo.lock: itoa 1.0.11",
            "Cargo.lock: libc 0.2.170",
            "Cargo.lock: pkgbuild 1.0.0",
            "Cargo.lock: serde_json 1.0.140",
            "crates/veredas-base/",
            "crates/veredas-core/",
            "crates/veredas-portal/",
            "docs/base-build.rs",
            "docs/core-lib.rs",
            "docs/deep.txt",
            "docs/example.txt",
            "docs/frag.in",
            "docs/frag.txt",
            "docs/outer.rs",
            "docs/ponte.rs",
            "docs/ponte.txt",
            "docs/shared.txt",
            "docs/unix.rs",
        ]
    );
    assert_eq!(
        keys("veredas-maquinar"),
        ["crates/veredas-maquinar/", "docs/bench.txt", "docs/dois.txt", "docs/helper.txt", "forks/dois/Cargo.toml", "forks/dois/lib/", "forks/um/Cargo.toml", "forks/um/lib/"]
    );
    let lock = std::fs::read_to_string(r.path().join("Cargo.lock")).unwrap();
    let charged = |component: &str, version: &str, inputs: &str| format!("{component} {version} is pinned by releases 2.0.0 and 2.0.1 at different build inputs ({inputs}): a version names everything its build reads");
    let portal = |i: &str| charged("veredas-portal", "2.0.0-alpha.3", i);
    let edit_lock = |from: &str, to: &str| {
        let l = lock.clone();
        let (from, to) = (from.to_string(), to.to_string());
        move |r: &Repo| {
            assert_eq!(l.matches(&from).count(), 1, "{from}");
            r.write("Cargo.lock", &l.replace(&from, &to));
        }
    };
    type Case = (&'static str, Box<dyn FnOnce(&Repo)>, Vec<String>);
    let cases: Vec<Case> = vec![
        ("a build dependency updated", Box::new(edit_lock("checksum = \"3333\"", "checksum = \"3434\"")), vec![portal("Cargo.lock: pkgbuild 1.0.0")]),
        ("a dependency on unix alone updated", Box::new(edit_lock("checksum = \"2222\"", "checksum = \"2323\"")), vec![portal("Cargo.lock: libc 0.2.170")]),
        ("a renamed dependency updated", Box::new(edit_lock("checksum = \"4444\"", "checksum = \"4545\"")), vec![portal("Cargo.lock: serde_json 1.0.140")]),
        ("the registry's itoa, named with its source", Box::new(edit_lock("checksum = \"1111\"", "checksum = \"1212\"")), vec![portal("Cargo.lock: itoa 1.0.11")]),
        ("the git itoa, which nothing uses", Box::new(edit_lock("#0123456789abcdef0123456789abcdef01234567", "#fedcba9876543210fedcba9876543210fedcba98")), vec![]),
        (
            "a member two paths away",
            Box::new(|r: &Repo| r.write("crates/veredas-base/src/lib.rs", "pub fn changed() {}\n")),
            vec![charged("veredas-base", "2.0.0", "crates/veredas-base/"), charged("veredas-core", "2.0.0", "crates/veredas-base/"), portal("crates/veredas-base/")],
        ),
        ("core's library source, outside its folder", Box::new(|r: &Repo| r.write("docs/core-lib.rs", "pub fn changed() {}\n")), vec![charged("veredas-core", "2.0.0", "docs/core-lib.rs"), portal("docs/core-lib.rs")]),
        ("the module named by cfg_attr", Box::new(|r: &Repo| r.write("docs/unix.rs", "pub fn changed() {}\n")), vec![portal("docs/unix.rs")]),
        ("the file a module under src/examples names", Box::new(|r: &Repo| r.write("docs/example.txt", "changed\n")), vec![portal("docs/example.txt")]),
        ("the file an outside module names", Box::new(|r: &Repo| r.write("docs/deep.txt", "changed\n")), vec![portal("docs/deep.txt")]),
        (
            "a fork a fork uses",
            Box::new(|r: &Repo| r.write("forks/dois/lib/src/lib.rs", "pub const D: &str = include_str!(\"../../../../docs/dois.txt\");\npub fn changed() {}\n")),
            vec![charged("veredas-maquinar", "2.1.0", "forks/dois/lib/")],
        ),
        (
            "an executable's source outside its folder",
            Box::new(|r: &Repo| r.write("docs/ponte.rs", "const P: &str = include_str!(\"ponte.txt\");\nfn main() { let _ = 1; }\n")),
            vec![portal("docs/ponte.rs")],
        ),
        (
            "a build script outside its folder, two paths away",
            Box::new(|r: &Repo| r.write("docs/base-build.rs", "fn main() { let _ = 1; }\n")),
            vec![charged("veredas-base", "2.0.0", "docs/base-build.rs"), charged("veredas-core", "2.0.0", "docs/base-build.rs"), portal("docs/base-build.rs")],
        ),
        ("a file a shipped source in `examples/` names", Box::new(|r: &Repo| r.write("docs/bench.txt", "changed\n")), vec![charged("veredas-maquinar", "2.1.0", "docs/bench.txt")]),
        ("a file a module beside that source names", Box::new(|r: &Repo| r.write("docs/helper.txt", "changed\n")), vec![charged("veredas-maquinar", "2.1.0", "docs/helper.txt")]),
        ("a file a module under `examples/` that `src/` names names", Box::new(|r: &Repo| r.write("docs/shared.txt", "changed\n")), vec![portal("docs/shared.txt")]),
        ("a file a Rust file without `.rs` names", Box::new(|r: &Repo| r.write("docs/frag.txt", "changed\n")), vec![portal("docs/frag.txt")]),
        ("a file a source outside the folders names", Box::new(|r: &Repo| r.write("docs/ponte.txt", "changed\n")), vec![portal("docs/ponte.txt")]),
        ("a file a fork's source names", Box::new(|r: &Repo| r.write("docs/dois.txt", "changed\n")), vec![charged("veredas-maquinar", "2.1.0", "docs/dois.txt")]),
    ];
    for (what, edit, want) in cases {
        edit(&r);
        let c = r.commit(what);
        r.git(&["reset", "-q", "--hard", &c1]);
        assert_eq!(manifest::check_together(r.path(), &[pins("2.0.0", &c1), pins("2.0.1", &c)]).unwrap(), want, "{what}");
    }
    // A package the lock file pins by path and no member uses by path (a `[patch]` to a folder):
    // the inputs do not read, by that reason.
    r.write("forks/serde_json/Cargo.toml", "[package]\nname = \"serde_json\"\nversion = \"1.0.140\"\nedition = \"2024\"\n");
    r.write("forks/serde_json/src/lib.rs", "\n");
    let t = std::fs::read_to_string(r.path().join("Cargo.toml")).unwrap();
    r.write("Cargo.toml", &format!("{t}\n[patch.crates-io]\nserde_json = {{ path = \"forks/serde_json\" }}\n"));
    r.write("Cargo.lock", &lock.replace(&format!("name = \"serde_json\"\nversion = \"1.0.140\"\nsource = \"{}\"\nchecksum = \"4444\"\n", "registry+https://github.com/rust-lang/crates.io-index"), "name = \"serde_json\"\nversion = \"1.0.140\"\n"));
    let c = r.commit("a patch to a folder");
    let held = manifest::components_at(r.path(), &c, &mut Vec::new()).unwrap();
    assert_eq!(held["veredas-portal"].inputs, Err("its Cargo.lock's serde_json 1.0.140 has no source and is no member a release builds from its folder".to_string()));
    // A `[replace]` to a folder, as cargo locks it (the registry's entry naming the package by path
    // in its place): followed to it, and refused by the same reason.
    r.write("Cargo.toml", &format!("{t}\n[replace]\n\"serde_json:1.0.140\" = {{ path = \"forks/serde_json\" }}\n"));
    r.write(
        "Cargo.lock",
        &lock
            .replace("checksum = \"4444\"\n", "checksum = \"4444\"\nreplace = \"serde_json 1.0.140\"\n\n[[package]]\nname = \"serde_json\"\nversion = \"1.0.140\"\n")
            .replace(" \"serde_json\",", " \"serde_json 1.0.140 (registry+https://github.com/rust-lang/crates.io-index)\","),
    );
    let c = r.commit("a replace to a folder");
    let held = manifest::components_at(r.path(), &c, &mut Vec::new()).unwrap();
    assert_eq!(held["veredas-portal"].inputs, Err("its Cargo.lock's serde_json 1.0.140 has no source and is no member a release builds from its folder".to_string()));
    // A root cargo configuration overriding packages by `paths`: the commit refused.
    r.write("Cargo.toml", &t);
    r.write("Cargo.lock", &lock);
    r.write(".cargo/config.toml", "paths = [\"forks/serde_json\"]\n");
    let c = r.commit("a paths override");
    let mut refusals = Vec::new();
    assert!(manifest::components_at(r.path(), &c, &mut refusals).unwrap().is_empty());
    assert_eq!(refusals, [format!("commit {c}: its .cargo/config.toml is a cargo configuration in the tree, which a release's build does not follow: what one sets reaches past what a version names")]);
}

#[test]
fn other_crate_types_and_a_git_dependency_are_named_as_inputs() {
    // A cdylib and a proc-macro starting outside their folders; a component using a git package,
    // named with its source (its commit), as the lock pins it.
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n");
    let git = "git+https://example.invalid/gitdep#0123456789abcdef0123456789abcdef01234567";
    r.write(
        "Cargo.lock",
        &format!(
            "version = 4\n\n[[package]]\nname = \"gitdep\"\nversion = \"0.1.0\"\nsource = \"{git}\"\n\n[[package]]\nname = \"veredas-git\"\nversion = \"2.0.0\"\ndependencies = [\n \"gitdep\",\n]\n\n\
             [[package]]\nname = \"veredas-macro\"\nversion = \"2.0.0\"\n\n[[package]]\nname = \"veredas-plug\"\nversion = \"2.0.0\"\n"
        ),
    );
    r.write("crates/veredas-plug/Cargo.toml", "[package]\nname = \"veredas-plug\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[lib]\ncrate-type = [\"cdylib\"]\npath = \"../../docs/plug.rs\"\n");
    r.write("crates/veredas-macro/Cargo.toml", "[package]\nname = \"veredas-macro\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[lib]\nproc-macro = true\npath = \"../../docs/macro.rs\"\n");
    r.write("crates/veredas-git/Cargo.toml", "[package]\nname = \"veredas-git\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[dependencies]\ngitdep = { git = \"https://example.invalid/gitdep\" }\n");
    r.write("crates/veredas-git/src/lib.rs", "\n");
    for (name, kind) in [("veredas-st", "staticlib"), ("veredas-dy", "dylib"), ("veredas-rl", "rlib")] {
        r.write(&format!("crates/{name}/Cargo.toml"), &format!("[package]\nname = \"{name}\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[lib]\ncrate-type = [\"{kind}\"]\npath = \"../../docs/{kind}.rs\"\n"));
        r.write(&format!("docs/{kind}.rs"), "\n");
    }
    r.write("docs/plug.rs", "\n");
    r.write("docs/macro.rs", "\n");
    let c = r.commit("other crate types, a git dependency");
    let mut refusals = Vec::new();
    let held = manifest::components_at(r.path(), &c, &mut refusals).unwrap();
    assert_eq!(refusals, Vec::<String>::new());
    let has = |n: &str, k: &str| held[n].inputs.as_ref().unwrap().0.contains_key(k);
    assert!(has("veredas-plug", "docs/plug.rs") && has("veredas-macro", "docs/macro.rs"));
    assert!(has("veredas-st", "docs/staticlib.rs") && has("veredas-dy", "docs/dylib.rs") && has("veredas-rl", "docs/rlib.rs"));
    assert_eq!(held["veredas-plug"].lib.as_deref(), Some("veredas_plug"), "a cdylib is the library the cut builds");
    assert!(has("veredas-git", &format!("Cargo.lock: gitdep 0.1.0 ({git})")), "{:?}", held["veredas-git"].inputs);
}

#[test]
fn the_lock_file_is_read_as_cargo_writes_it() {
    // Two git packages of one name and version (cargo's dependency strings name their sources
    // without the commit), a registry package `[replace]`d by a git one, and a fork one component
    // uses by path that a `[patch]` also puts in place of the registry's, for a crate it uses.
    let r = Repo::new();
    r.write(
        "Cargo.toml",
        "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"forks\"]\n\n[patch.crates-io]\nitoa = { path = \"forks/itoa\" }\n",
    );
    let reg = "registry+https://github.com/rust-lang/crates.io-index";
    let (a, b, ryu) = ("git+https://example.invalid/A", "git+https://example.invalid/B", "git+https://example.invalid/ryu");
    let _ = &ryu;
    let (ca, cb, cr) = ("a".repeat(40), "b".repeat(40), "c".repeat(40));
    let lock = format!(
        "version = 4\n\n[[package]]\nname = \"gitdep\"\nversion = \"0.1.0\"\nsource = \"{a}#{ca}\"\n\n[[package]]\nname = \"gitdep\"\nversion = \"0.1.0\"\nsource = \"{b}#{cb}\"\n\n\
         [[package]]\nname = \"itoa\"\nversion = \"1.0.18\"\n\n[[package]]\nname = \"ryu\"\nversion = \"1.0.0\"\nsource = \"{reg}\"\nchecksum = \"7777\"\nreplace = \"ryu 1.0.0 ({ryu})\"\n\n\
         [[package]]\nname = \"ryu\"\nversion = \"1.0.0\"\nsource = \"{ryu}#{cr}\"\n\n[[package]]\nname = \"serde_json\"\nversion = \"1.0.154\"\nsource = \"{reg}\"\nchecksum = \"8888\"\ndependencies = [\n \"itoa\",\n]\n\n\
         [[package]]\nname = \"veredas-a\"\nversion = \"2.0.0\"\ndependencies = [\n \"gitdep 0.1.0 ({a})\",\n]\n\n[[package]]\nname = \"veredas-b\"\nversion = \"2.0.0\"\ndependencies = [\n \"gitdep 0.1.0 ({b})\",\n]\n\n\
         [[package]]\nname = \"veredas-c\"\nversion = \"2.0.0\"\ndependencies = [\n \"ryu 1.0.0 ({reg})\",\n]\n\n[[package]]\nname = \"veredas-d\"\nversion = \"2.0.0\"\ndependencies = [\n \"itoa\",\n \"serde_json\",\n]\n"
    );
    r.write("Cargo.lock", &lock);
    for (name, deps) in [
        ("veredas-a", "gitdep = { git = \"https://example.invalid/A\" }"),
        ("veredas-b", "gitdep = { git = \"https://example.invalid/B\" }"),
        ("veredas-d", "itoa = { path = \"../../forks/itoa\" }\nserde_json = \"=1.0.154\""),
    ] {
        r.write(&format!("crates/{name}/Cargo.toml"), &format!("[package]\nname = \"{name}\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[dependencies]\n{deps}\n"));
        r.write(&format!("crates/{name}/src/lib.rs"), "\n");
    }
    r.write("forks/itoa/Cargo.toml", "[workspace]\n\n[package]\nname = \"itoa\"\nversion = \"1.0.18\"\nedition = \"2024\"\n");
    r.write("forks/itoa/src/lib.rs", "\n");
    let c = r.commit("the lock's spellings");
    let mut refusals = Vec::new();
    let held = manifest::components_at(r.path(), &c, &mut refusals).unwrap();
    assert_eq!(refusals, Vec::<String>::new());
    let keys = |n: &str| -> Vec<String> { held[n].inputs.as_ref().unwrap_or_else(|e| panic!("{n}: {e}")).0.keys().filter(|k| k.starts_with("Cargo.lock") || k.starts_with("forks")).cloned().collect() };
    assert_eq!(keys("veredas-a"), [format!("Cargo.lock: gitdep 0.1.0 ({a}#{ca})")]);
    assert_eq!(keys("veredas-b"), [format!("Cargo.lock: gitdep 0.1.0 ({b}#{cb})")]);
    assert_eq!(keys("veredas-d"), ["Cargo.lock: serde_json 1.0.154", "forks/itoa/", "forks/itoa/Cargo.toml"], "the fork is a workspace of its own: its manifest is its workspace's");
    // Cargo takes `[replace]` and `[patch]` apart: the git replacement in a workspace of its own,
    // then one the lock does not pin, refused by its reason.
    let q = Repo::new();
    q.write("Cargo.toml", "[workspace]\nmembers = [\"crates/*\"]\n\n[replace]\n\"ryu:1.0.0\" = { git = \"https://example.invalid/ryu\" }\n");
    q.write("crates/veredas-c/Cargo.toml", "[package]\nname = \"veredas-c\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[dependencies]\nryu = \"1\"\n");
    q.write("crates/veredas-c/src/lib.rs", "\n");
    let qlock = |replace: &str| {
        format!(
            "version = 4\n\n[[package]]\nname = \"ryu\"\nversion = \"1.0.0\"\nsource = \"{reg}\"\nchecksum = \"7777\"\nreplace = \"{replace}\"\n\n\
             [[package]]\nname = \"ryu\"\nversion = \"1.0.0\"\nsource = \"{ryu}#{cr}\"\n\n[[package]]\nname = \"veredas-c\"\nversion = \"2.0.0\"\ndependencies = [\n \"ryu 1.0.0 ({reg})\",\n]\n"
        )
    };
    q.write("Cargo.lock", &qlock(&format!("ryu 1.0.0 ({ryu})")));
    let qc = q.commit("a git replacement");
    let held_q = manifest::components_at(q.path(), &qc, &mut Vec::new()).unwrap();
    let qkeys: Vec<String> = held_q["veredas-c"].inputs.as_ref().unwrap().0.keys().filter(|k| k.starts_with("Cargo.lock")).cloned().collect();
    assert_eq!(qkeys, ["Cargo.lock: ryu 1.0.0".to_string(), format!("Cargo.lock: ryu 1.0.0 ({ryu}#{cr})")]);
    q.write("Cargo.lock", &qlock("ryu 1.0.0 (git+https://example.invalid/nope)"));
    let qc2 = q.commit("a replacement not pinned");
    let held_q = manifest::components_at(q.path(), &qc2, &mut Vec::new()).unwrap();
    assert_eq!(held_q["veredas-c"].inputs, Err("its Cargo.lock replaces ryu 1.0.0 by `ryu 1.0.0 (git+https://example.invalid/nope)`, which it does not pin".to_string()));
    // A cargo configuration without its extension overriding by `paths`: refused by its reason.
    r.write(".cargo/config", "paths = [\"forks/itoa\"]\n");
    let c3 = r.commit("a paths override without its extension");
    let mut refusals = Vec::new();
    assert!(manifest::components_at(r.path(), &c3, &mut refusals).unwrap().is_empty());
    assert_eq!(refusals, [format!("commit {c3}: its .cargo/config is a cargo configuration in the tree, which a release's build does not follow: what one sets reaches past what a version names")]);
}

#[test]
fn a_root_cargo_configuration_is_refused_whatever_it_sets() {
    // Sources, a wrapper, a linker, patches, an `include` of another file, a linker named by a flag,
    // flags alone: a configuration in the tree reaches past what a version names, and is refused
    // whole (with or without its extension).
    for (file, config) in [
        (".cargo/config.toml", "[source.crates-io]\nreplace-with = \"tree\"\n[source.tree]\ndirectory = \"third_party\"\n"),
        (".cargo/config.toml", "[build]\nrustc-wrapper = \"tools/wrap.sh\"\n"),
        (".cargo/config.toml", "[target.x86_64-unknown-linux-gnu]\nlinker = \"tools/ld.sh\"\n"),
        (".cargo/config.toml", "include = [\"../tools/more.toml\"]\n"),
        (".cargo/config.toml", "[build]\nrustflags = [\"-C\", \"linker=tools/enlace.sh\"]\n"),
        (".cargo/config.toml", "[build]\nrustflags = [\"-Cdebuginfo=0\"]\n"),
        (".cargo/config", "[build]\nrustc = \"tools/rustc.sh\"\n"),
        (".cargo/config", "[host]\nlinker = \"tools/ld.sh\"\n"),
    ] {
        let r = wider();
        r.write(file, config);
        let c = r.commit(config);
        let mut refusals = Vec::new();
        assert!(manifest::components_at(r.path(), &c, &mut refusals).unwrap().is_empty(), "{config}");
        assert_eq!(refusals, [format!("commit {c}: its {file} is a cargo configuration in the tree, which a release's build does not follow: what one sets reaches past what a version names")], "{config}");
    }
}


#[test]
fn round_nineteen_raw_identifiers_a_shebang_and_what_a_data_file_names() {
    // A raw identifier is the same macro or attribute; a first line that is a shebang is dropped,
    // as rustc drops it; what a data file (include_str!) holds is never read as Rust.
    let src = "#!/usr/bin/env -S echo \"hi\nconst A: &str = r#include_str!(\"raw.txt\");\n#[r#path = \"raw.rs\"]\nmod m;\n// \"\n";
    assert_eq!(alforge::inputs::literals(src), ["raw.txt", "raw.rs"]);
    assert_eq!(alforge::inputs::literals("#![allow(dead_code)]\ninclude!(\"kept.rs\");\n"), ["kept.rs"], "an inner attribute is no shebang");
    assert_eq!(alforge::inputs::literals("const X: &str = include_str!(\"/etc/hostname\");\n"), ["/etc/hostname"]);
}

#[test]
fn round_nineteen_the_lock_by_requirement_a_package_by_path_through_the_lock_and_a_later_conflict() {
    let reg = "registry+https://github.com/rust-lang/crates.io-index";
    // Portal ships with itoa 1 and tests with itoa 0.4 (under another name); a fork of serde_json's
    // itoa... is not needed: a member reached only through the lock (a [patch] to a member) is
    // named by its folder.
    let r = Repo::new();
    r.write("Cargo.toml", "[workspace]\nmembers = [\"crates/veredas-portal\", \"tools/itoa\"]\n\n[patch.crates-io]\nitoa = { path = \"tools/itoa\" }\n");
    r.write(
        "crates/veredas-portal/Cargo.toml",
        "[package]\nname = \"veredas-portal\"\nversion = \"2.0.0\"\nedition = \"2024\"\n\n[dependencies]\nserde_json = \"1\"\nryu = \"1\"\n\n[dev-dependencies]\nryu_old = { package = \"ryu\", version = \"0.2\" }\n",
    );
    r.write("crates/veredas-portal/src/lib.rs", "\n");
    r.write("tools/itoa/Cargo.toml", "[package]\nname = \"itoa\"\nversion = \"1.0.18\"\nedition = \"2024\"\n");
    r.write("tools/itoa/src/lib.rs", "\n");
    let lock = |old_ryu: &str| {
        format!(
            "version = 4\n\n[[package]]\nname = \"itoa\"\nversion = \"1.0.18\"\n\n[[package]]\nname = \"ryu\"\nversion = \"0.2.8\"\nsource = \"{reg}\"\nchecksum = \"{old_ryu}\"\n\n\
             [[package]]\nname = \"ryu\"\nversion = \"1.0.20\"\nsource = \"{reg}\"\nchecksum = \"6666\"\n\n[[package]]\nname = \"serde_json\"\nversion = \"1.0.140\"\nsource = \"{reg}\"\nchecksum = \"5555\"\ndependencies = [\n \"itoa\",\n]\n\n\
             [[package]]\nname = \"veredas-portal\"\nversion = \"2.0.0\"\ndependencies = [\n \"ryu 0.2.8\",\n \"ryu 1.0.20\",\n \"serde_json\",\n]\n"
        )
    };
    r.write("Cargo.lock", &lock("7777"));
    let c1 = r.commit("a test's other ryu, a patched itoa");
    let held = manifest::components_at(r.path(), &c1, &mut Vec::new()).unwrap();
    let keys: Vec<String> = held["veredas-portal"].inputs.as_ref().unwrap().0.keys().filter(|k| k.starts_with("Cargo.lock") || k.starts_with("tools")).cloned().collect();
    assert_eq!(keys, ["Cargo.lock: ryu 1.0.20", "Cargo.lock: serde_json 1.0.140", "tools/itoa/"], "the test's ryu 0.2 not charged; the patched itoa by its folder");
    // The test's ryu updated: no charge.
    r.write("Cargo.lock", &lock("7878"));
    let c2 = r.commit("the test's ryu updated");
    let pin = |release: &str, commit: &str| -> (PathBuf, Manifest) {
        let text = format!(
            "format = \"veredas-release\"\nversion = 1\nrelease = \"{release}\"\ncommit = \"{commit}\"\ntarget = \"x86_64-unknown-linux-gnu\"\ntoolchain = \"rustc 1.100.0\"\n\n\
             [[component]]\nname = \"veredas-portal\"\nversion = \"2.0.0\"\n\n[tested]\nsuite = \"cargo nextest run --workspace\"\ntogether = \"the bundle's bench and supervisão on its plant\"\n"
        );
        (PathBuf::from(format!("{release}.toml")), manifest::load(&text).unwrap())
    };
    assert_eq!(manifest::check_together(r.path(), &[pin("2.0.0", &c1), pin("2.0.1", &c2)]).unwrap(), Vec::<String>::new());
    // A lock entry of the member at another version: refused as a lock that is not this workspace's.
    r.write("Cargo.lock", &lock("7777").replace("name = \"veredas-portal\"\nversion = \"2.0.0\"", "name = \"veredas-portal\"\nversion = \"1.9.0\""));
    let stale = r.commit("a stale lock");
    let held = manifest::components_at(r.path(), &stale, &mut Vec::new()).unwrap();
    assert_eq!(held["veredas-portal"].inputs, Err("its Cargo.lock does not pin veredas-portal 2.0.0: the lock file is not this workspace's".to_string()));
    // An unreadable first pin hides no conflict between the later ones.
    r.write("Cargo.lock", &lock("7777"));
    r.write("crates/veredas-portal/src/lib.rs", "pub fn changed() {}\n");
    let c4 = r.commit("portal changed");
    let got = manifest::check_together(r.path(), &[pin("2.0.0", &stale), pin("2.0.1", &c1), pin("2.0.2", &c4)]).unwrap();
    assert_eq!(
        got,
        [
            "veredas-portal 2.0.0 is pinned by releases 2.0.0 and 2.0.1, and its build inputs at 2.0.0's commit do not read: its Cargo.lock does not pin veredas-portal 2.0.0: the lock file is not this workspace's".to_string(),
            "veredas-portal 2.0.0 is pinned by releases 2.0.1 and 2.0.2 at different build inputs (crates/veredas-portal/): a version names everything its build reads".to_string(),
        ]
    );
    // A name holding a control character: the commit refused by that path.
    r.write("crates/veredas-portal/src/a\tb.rs", "\n");
    let tab = r.commit("a tab in a name");
    let mut refusals = Vec::new();
    manifest::components_at(r.path(), &tab, &mut refusals).unwrap();
    assert_eq!(refusals, [format!("commit {tab}: the path \"crates/veredas-portal/src/a\\tb.rs\" holds a control character: a release's tree names its files without one")]);
}

#[test]
fn round_nineteen_an_absolute_path_is_refused_and_a_data_file_names_nothing() {
    let r = wider();
    r.write("crates/veredas-portal/src/bin/porteiro.rs", "const H: &str = include_str!(\"/etc/hostname\");\nfn main() {}\n");
    let c = r.commit("an absolute include");
    let held = manifest::components_at(r.path(), &c, &mut Vec::new()).unwrap();
    assert_eq!(held["veredas-portal"].inputs, Err("crates/veredas-portal/src/bin/porteiro.rs names `/etc/hostname`, outside the commit's tree".to_string()));
    // What a data file holds is not read: a file it names in Rust's form charges nothing.
    let r = wider();
    r.write("docs/notes.txt", "include_str!(\"../named-by-data.txt\")\n");
    r.write("named-by-data.txt", "x\n");
    let c = r.commit("a data file naming a file");
    let held = manifest::components_at(r.path(), &c, &mut Vec::new()).unwrap();
    assert!(!held["veredas-portal"].inputs.as_ref().unwrap().0.contains_key("named-by-data.txt"));
}
