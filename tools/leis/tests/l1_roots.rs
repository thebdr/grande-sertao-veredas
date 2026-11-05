//! L1 (C-001): every crate under a root is a listed member or excluded; every member, and every
//! crate found, lives directly under `crates/` or `tools/`.

mod common;

use common::*;
use leis::Suite;

#[test]
fn l1_a_member_outside_the_roots_is_named() {
    let s = suite(&ws(&["demos/conveyor"]), &[], &[]);
    assert_eq!(strings(leis::roots(&s)), vec!["L1 demos/conveyor: a member lives directly under crates/ or tools/"]);
}

#[test]
fn l1_a_listed_member_without_a_manifest_is_named() {
    let s = suite(&ws(&["crates/*", "tools/leis"]), &[("tools/leis", LEIS)], &[]);
    assert_eq!(
        strings(leis::roots(&s)),
        vec!["L1 crates/*: listed, but no Cargo.toml there (a glob is not a member; list each one)"]
    );
}

#[test]
fn l1_a_crate_that_is_not_listed_is_named() {
    let stray = pkg("veredas-portal", "2.0.0");
    let s = suite(&ws(&["tools/leis"]), &[("tools/leis", LEIS), ("crates/veredas-portal", &stray)], &[]);
    assert_eq!(
        strings(leis::roots(&s)),
        vec!["L1 crates/veredas-portal: has a Cargo.toml but is not a workspace member (list it, or exclude it)"]
    );
}

#[test]
fn l1_an_excluded_crate_is_not_a_member() {
    let fork = pkg("ironcalc", "0.5.0");
    let workspace = "[workspace]\nmembers = [\"tools/leis\"]\nexclude = [\"crates/ironcalc\"]\n[workspace.package]\nedition = \"2024\"\n";
    let s = suite(workspace, &[("tools/leis", LEIS), ("crates/ironcalc", &fork)], &[]);
    assert_eq!(strings(leis::roots(&s)), NONE);
    assert_eq!(s.members.len(), 1, "an excluded crate is not a member, so no other law sees it");
}

#[test]
fn l1_an_excluded_folder_covers_what_is_beneath_it() {
    let fork = pkg("ironcalc", "0.5.0");
    let base = pkg("ironcalc-base", "0.5.0");
    let workspace = "[workspace]\nmembers = [\"tools/leis\"]\nexclude = [\"crates/ironcalc\"]\n[workspace.package]\nedition = \"2024\"\n";
    let s = suite(
        workspace,
        &[("tools/leis", LEIS), ("crates/ironcalc", &fork), ("crates/ironcalc/base", &base)],
        &[],
    );
    assert_eq!(strings(leis::roots(&s)), NONE);
    // and a sibling with a prefix in common is not covered
    let cousin = pkg("ironcalc-py", "0.5.0");
    let s = suite(workspace, &[("tools/leis", LEIS), ("crates/ironcalc-py", &cousin)], &[]);
    assert_eq!(
        strings(leis::roots(&s)),
        vec!["L1 crates/ironcalc-py: has a Cargo.toml but is not a workspace member (list it, or exclude it)"]
    );
}

#[test]
fn l1_a_suite_crate_is_never_excluded() {
    // by its name, by its folder, and behind an excluded parent folder
    let abaco = pkg("veredas-abaco", "2.0.0");
    let disguised = pkg("veredas-portal", "2.0.0");
    let orcl = pkg("orcl-std", "2.0.0");
    let workspace = "[workspace]\nmembers = [\"tools/leis\"]\nexclude = [\"crates/veredas-abaco\", \"crates/gateway\", \"crates/lang\"]\n[workspace.package]\nedition = \"2024\"\n";
    let s = suite(
        workspace,
        &[("tools/leis", LEIS), ("crates/veredas-abaco", &abaco), ("crates/gateway", &disguised), ("crates/lang/orcl-std", &orcl)],
        &[],
    );
    assert_eq!(
        strings(leis::roots(&s)),
        vec![
            "L1 crates/veredas-abaco: a veredas-* or orcl-* crate is the suite's; it is a member, never excluded",
            "L1 crates/gateway: a veredas-* or orcl-* crate is the suite's; it is a member, never excluded",
            "L1 crates/lang/orcl-std: a veredas-* or orcl-* crate is the suite's; it is a member, never excluded",
        ]
    );
    // the whole root excluded hides nothing of the suite's
    let workspace = "[workspace]\nmembers = [\"tools/leis\"]\nexclude = [\"crates\"]\n[workspace.package]\nedition = \"2024\"\n";
    let s = suite(workspace, &[("tools/leis", LEIS), ("crates/veredas-abaco", &abaco)], &[]);
    assert_eq!(
        strings(leis::roots(&s)),
        vec!["L1 crates/veredas-abaco: a veredas-* or orcl-* crate is the suite's; it is a member, never excluded"]
    );
}

#[test]
fn l1_a_suite_crate_outside_the_roots_is_named() {
    // excluded elsewhere, parked elsewhere with its own [workspace], or simply unlisted elsewhere:
    // a suite crate is a member under a root or nothing (refute round 3)
    let portal = pkg("veredas-portal", "1.0.0");
    let own = format!("{}[workspace]\n", pkg("veredas-aranha", "2.0.0"));
    let maquinar = pkg("veredas-maquinar", "2.0.0");
    let conveyor = pkg("conveyor", "0.1.0");
    let workspace = "[workspace]\nmembers = [\"tools/leis\"]\nexclude = [\"apps/veredas-portal\"]\n[workspace.package]\nedition = \"2024\"\n";
    let s = suite(
        workspace,
        &[
            ("tools/leis", LEIS),
            ("apps/veredas-portal", &portal),
            ("bench/veredas-aranha", &own),
            ("veredas-maquinar", &maquinar),
            ("demos/conveyor", &conveyor),
        ],
        &[],
    );
    assert_eq!(
        strings(leis::roots(&s)),
        vec![
            "L1 apps/veredas-portal: a veredas-* or orcl-* crate is the suite's; it is a member, never excluded",
            "L1 bench/veredas-aranha: a veredas-* or orcl-* crate is the suite's; it lives under crates/ as a member",
            "L1 veredas-maquinar: a veredas-* or orcl-* crate is the suite's; it lives under crates/ as a member",
        ],
        "a crate elsewhere that is not the suite's (demos/conveyor) is nobody's business"
    );
}

#[test]
fn l1_an_underscore_spelling_is_the_same_suite_crate() {
    let disguised = pkg("veredas_portal", "2.0.0");
    let lang = pkg("orcl_std", "2.0.0");
    let workspace = "[workspace]\nmembers = [\"tools/leis\"]\nexclude = [\"crates/gateway\", \"crates/veredas_portal\", \"crates/lang\"]\n[workspace.package]\nedition = \"2024\"\n";
    let s = suite(
        workspace,
        &[("tools/leis", LEIS), ("crates/gateway", &disguised), ("crates/veredas_portal", &disguised), ("crates/lang/orcl_std", &lang)],
        &[],
    );
    assert_eq!(
        strings(leis::roots(&s)),
        vec![
            "L1 crates/gateway: a veredas-* or orcl-* crate is the suite's; it is a member, never excluded",
            "L1 crates/veredas_portal: a veredas-* or orcl-* crate is the suite's; it is a member, never excluded",
            "L1 crates/lang/orcl_std: a veredas-* or orcl-* crate is the suite's; it is a member, never excluded",
        ]
    );
}

#[test]
fn l1_the_reader_scans_the_whole_repository_for_suite_crates() {
    let portal = pkg("veredas-portal", "2.0.0");
    let conveyor = pkg("conveyor", "0.1.0");
    let parked = format!("{}[workspace]\n", pkg("veredas-parked", "1.0.0"));
    let tree = Tree::new(
        "l1-elsewhere",
        &[
            (".gitignore", "target/\n**/node_modules\n"),
            ("Cargo.toml", "[workspace]\nmembers = [\"tools/leis\"]\nexclude = [\"apps/veredas-portal\"]\n[workspace.package]\nedition = \"2024\"\n"),
            ("tools/leis/Cargo.toml", LEIS),
            ("apps/veredas-portal/Cargo.toml", &portal),
            ("demos/conveyor/Cargo.toml", &conveyor),
            ("docs/veredas-plan.md", "# the plan\n"),
            (".parked/veredas-parked/Cargo.toml", &parked),
            ("docs/.old/veredas-parked/Cargo.toml", &parked),
            ("target/veredas-built/Cargo.toml", &parked),
            ("web/node_modules/veredas-built/Cargo.toml", &parked),
        ],
    );
    let s = Suite::read(tree.path()).expect("the tree reads");
    assert_eq!(
        s.found,
        vec![".parked/veredas-parked", "apps/veredas-portal", "demos/conveyor", "docs/.old/veredas-parked", "tools/leis"],
        "the root's own manifest left out, what git ignores left out, a hidden folder seen (refute round 4)"
    );
    assert_eq!(
        strings(leis::roots(&s)),
        vec![
            "L1 .parked/veredas-parked: a veredas-* or orcl-* crate is the suite's; it lives under crates/ as a member",
            "L1 apps/veredas-portal: a veredas-* or orcl-* crate is the suite's; it is a member, never excluded",
            "L1 docs/.old/veredas-parked: a veredas-* or orcl-* crate is the suite's; it lives under crates/ as a member",
        ]
    );
}

#[test]
fn l1_the_reader_sees_what_git_would_take_not_what_a_name_suggests() {
    // a nested .gitignore un-ignores target/, and a forced add takes an ignored path: both are
    // in the repository, so both are seen (refute round 6: a fixed skip list was a way out);
    // what git ignores is left out, and a tree with no repository is refused
    let parked = format!("{}[workspace]\n", pkg("veredas-parked", "1.0.0"));
    let tree = Tree::new(
        "l1-git-listing",
        &[
            (".gitignore", "target/\n**/node_modules\n"),
            ("Cargo.toml", &ws(&["tools/leis"])),
            ("tools/leis/Cargo.toml", LEIS),
            ("bench/.gitignore", "!target/\n"),
            ("bench/target/veredas-parked/Cargo.toml", &parked),
            ("target/veredas-built/Cargo.toml", &parked),
            ("web/target/veredas-forced/Cargo.toml", &parked),
        ],
    );
    let forced = std::process::Command::new("git")
        .arg("-C")
        .arg(tree.path())
        .args(["add", "-f", "web/target/veredas-forced/Cargo.toml"])
        .status()
        .expect("git runs");
    assert!(forced.success());
    let s = Suite::read(tree.path()).expect("the tree reads");
    assert_eq!(s.found, vec!["bench/target/veredas-parked", "tools/leis", "web/target/veredas-forced"]);
    assert_eq!(
        strings(leis::roots(&s)),
        vec![
            "L1 bench/target/veredas-parked: a veredas-* or orcl-* crate is the suite's; it lives under crates/ as a member",
            "L1 web/target/veredas-forced: a veredas-* or orcl-* crate is the suite's; it lives under crates/ as a member",
        ]
    );
    let bare = Tree::new("l1-no-git", &[("Cargo.toml", &ws(&["tools/leis"])), ("tools/leis/Cargo.toml", LEIS)]).without_git();
    assert!(Suite::read(bare.path()).is_err(), "the laws read a repository, nothing else");
}

#[test]
fn l1_the_reader_takes_a_manifest_in_any_case_and_a_submodule_by_its_folder() {
    // `cargo.toml` is a manifest cargo builds on a filesystem that ignores case, and a submodule
    // is a folder the repository holds whose files it never lists (refute round 7)
    let parked = format!("{}[workspace]\n", pkg("veredas-portal", "1.0.0"));
    let sub = format!("{}[workspace]\n", pkg("veredas-aranha", "1.0.0"));
    let git = |root: &std::path::Path, args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["-c", "user.name=leis", "-c", "user.email=leis@veredas", "-c", "protocol.file.allow=always"])
            .args(args)
            .output()
            .expect("git runs");
        assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let source = Tree::new("l1-subrepo", &[("Cargo.toml", &sub), ("src/lib.rs", "")]);
    git(source.path(), &["add", "-A"]);
    git(source.path(), &["commit", "-q", "-m", "the aranha, elsewhere"]);
    let tree = Tree::new(
        "l1-case-and-submodule",
        &[("Cargo.toml", &ws(&["tools/leis"])), ("tools/leis/Cargo.toml", LEIS), ("apps/veredas-portal/cargo.toml", &parked)],
    );
    let url = format!("file:///{}", source.path().display().to_string().replace('\\', "/"));
    git(tree.path(), &["submodule", "-q", "add", &url, "vendor/veredas-aranha"]);
    // and a submodule pointer with no checkout at all, known by its folder alone
    let sha = git(source.path(), &["rev-parse", "HEAD"]);
    git(tree.path(), &["update-index", "--add", "--cacheinfo", &format!("160000,{sha},bench/orcl-bancada")]);
    let s = Suite::read(tree.path()).expect("the tree reads");
    assert_eq!(s.found, vec!["apps/veredas-portal", "bench/orcl-bancada", "tools/leis", "vendor/veredas-aranha"]);
    assert_eq!(
        strings(leis::roots(&s)),
        vec![
            "L1 apps/veredas-portal: a veredas-* or orcl-* crate is the suite's; it lives under crates/ as a member",
            "L1 bench/orcl-bancada: a veredas-* or orcl-* crate is the suite's; it lives under crates/ as a member",
            "L1 vendor/veredas-aranha: a veredas-* or orcl-* crate is the suite's; it lives under crates/ as a member",
        ]
    );
}

#[test]
fn l1_two_spellings_of_the_manifest_in_one_folder_are_refused() {
    // on a filesystem that tells case apart (the CI runner), a decoy cargo.toml beside the real
    // Cargo.toml would be a different file, and the reader must neither pick the decoy nor stay
    // silent (refute round 8). Both spellings are put in git's index, which lists them on every
    // filesystem; on disk a case-blind one keeps a single file
    let real = format!("{}[workspace]\n", pkg("veredas-portal", "1.0.0"));
    let decoy = "# not the manifest cargo reads where case is told apart\n[package]\nname = \"serde\"\nversion = \"1.0.0\"\n";
    let tree = Tree::new(
        "l1-case-decoy",
        &[("Cargo.toml", &ws(&["tools/leis"])), ("tools/leis/Cargo.toml", LEIS), ("apps/gw/cargo.toml", decoy), ("apps/gw/Cargo.toml", &real), ("decoy.txt", decoy)],
    );
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git").arg("-C").arg(tree.path()).args(args).output().expect("git runs");
        assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    let real_blob = git(&["hash-object", "-w", "apps/gw/Cargo.toml"]);
    let decoy_blob = git(&["hash-object", "-w", "decoy.txt"]);
    std::fs::remove_file(tree.path().join("decoy.txt")).expect("the helper file goes");
    git(&["update-index", "--add", "--cacheinfo", &format!("100644,{real_blob},apps/gw/Cargo.toml")]);
    git(&["update-index", "--add", "--cacheinfo", &format!("100644,{decoy_blob},apps/gw/cargo.toml")]);
    let s = Suite::read(tree.path()).expect("the tree reads");
    assert_eq!(s.found, vec!["apps/gw", "tools/leis"]);
    assert_eq!(s.ambiguous, vec!["apps/gw"]);
    assert_eq!(s.others.iter().map(|o| o.name.as_str()).collect::<Vec<_>>(), vec!["veredas-portal"], "the exact Cargo.toml is the text taken");
    assert_eq!(
        strings(leis::roots(&s)),
        vec![
            "L1 apps/gw: two manifests differ only in case; which one cargo reads depends on the filesystem",
            "L1 apps/gw: a veredas-* or orcl-* crate is the suite's; it lives under crates/ as a member",
        ]
    );
}

#[test]
fn l1_a_nested_folder_is_not_a_root() {
    let deep = pkg("veredas-abaco", "2.0.0");
    let s = suite(&ws(&["crates/abaco/veredas-abaco"]), &[("crates/abaco/veredas-abaco", &deep)], &[]);
    assert_eq!(
        strings(leis::roots(&s)),
        vec![
            "L1 crates/abaco/veredas-abaco: a member lives directly under crates/ or tools/",
            "L1 crates/abaco/veredas-abaco: a crate lives directly under crates/ or tools/ (a nested folder is not a root; exclude what is not the suite's)",
        ]
    );
}

#[test]
fn l1_a_nested_unlisted_crate_is_named() {
    let deep = pkg("veredas-abaco", "2.0.0");
    let at_root = pkg("crates", "0.0.0");
    let s = suite(
        &ws(&["tools/leis"]),
        &[("tools/leis", LEIS), ("crates/abaco/veredas-abaco", &deep), ("crates", &at_root)],
        &[],
    );
    assert_eq!(
        strings(leis::roots(&s)),
        vec![
            "L1 crates/abaco/veredas-abaco: a veredas-* or orcl-* crate is the suite's; it lives under crates/ as a member",
            "L1 crates: a crate lives directly under crates/ or tools/ (a nested folder is not a root; exclude what is not the suite's)",
        ]
    );
}

#[test]
fn l1_the_reader_finds_crates_at_any_depth() {
    let deep = pkg("veredas-abaco", "2.0.0");
    let at_root = pkg("crates", "0.0.0");
    let inner = pkg("inner", "0.1.0");
    let built = pkg("built", "0.1.0");
    let hidden = pkg("hidden", "0.1.0");
    let tree = Tree::new(
        "l1-depth",
        &[
            (".gitignore", "target/\n**/node_modules\n"),
            ("Cargo.toml", &ws(&["tools/leis"])),
            ("tools/leis/Cargo.toml", LEIS),
            ("crates/abaco/veredas-abaco/Cargo.toml", &deep),
            ("crates/Cargo.toml", &at_root),
            ("tools/leis-extra/inner/Cargo.toml", &inner),
            ("crates/veredas-abaco/target/debug/build/Cargo.toml", &built),
            ("tools/.cache/Cargo.toml", &hidden),
            ("crates/veredas-abaco/web/node_modules/pkg/Cargo.toml", &built),
        ],
    );
    let s = Suite::read(tree.path()).expect("the tree reads");
    assert_eq!(
        s.found,
        vec!["crates", "crates/abaco/veredas-abaco", "tools/.cache", "tools/leis", "tools/leis-extra/inner"],
        "every manifest under a root at any depth, a hidden folder included, none where git ignores"
    );
    assert_eq!(
        strings(leis::roots(&s)),
        vec![
            "L1 crates: a crate lives directly under crates/ or tools/ (a nested folder is not a root; exclude what is not the suite's)",
            "L1 crates/abaco/veredas-abaco: a veredas-* or orcl-* crate is the suite's; it lives under crates/ as a member",
            "L1 tools/.cache: has a Cargo.toml but is not a workspace member (list it, or exclude it)",
            "L1 tools/leis-extra/inner: a crate lives directly under crates/ or tools/ (a nested folder is not a root; exclude what is not the suite's)",
        ]
    );
}

#[test]
fn l1_the_reader_sees_an_exclusion_as_cargo_does() {
    let fork = pkg("ironcalc", "0.5.0");
    let base = pkg("ironcalc-base", "0.5.0");
    let tree = Tree::new(
        "l1-exclude",
        &[
            ("Cargo.toml", "[workspace]\nmembers = [\"tools/leis\"]\nexclude = [\"crates/ironcalc\"]\n[workspace.package]\nedition = \"2024\"\n"),
            ("tools/leis/Cargo.toml", LEIS),
            ("crates/ironcalc/Cargo.toml", &fork),
            ("crates/ironcalc/base/Cargo.toml", &base),
        ],
    );
    let s = Suite::read(tree.path()).expect("the tree reads");
    assert_eq!(s.found, vec!["crates/ironcalc", "crates/ironcalc/base", "tools/leis"]);
    assert_eq!(strings(leis::roots(&s)), NONE);
}
