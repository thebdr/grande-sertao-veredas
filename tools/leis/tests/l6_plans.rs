//! L6 (C-006): a component's plan exists, not blank, from its first commit; the language's crates
//! share `docs/orcl/README.md`; the suite's own plan is nobody's.

mod common;

use common::*;
use leis::Suite;

#[test]
fn l6_a_component_without_its_plan_is_named() {
    let abaco = pkg("veredas-abaco", "2.0.0");
    let s = suite(&ws(&["crates/veredas-abaco"]), &[("crates/veredas-abaco", &abaco)], &[("docs/portal-plan.md", "# portal\n")]);
    assert_eq!(
        strings(leis::plans(&s)),
        vec!["L6 crates/veredas-abaco: no docs/abaco-plan.md (a role nobody covers, written before the code)"]
    );
}

#[test]
fn l6_an_empty_plan_is_named() {
    let abaco = pkg("veredas-abaco", "2.0.0");
    let s = suite(&ws(&["crates/veredas-abaco"]), &[("crates/veredas-abaco", &abaco)], &[("docs/abaco-plan.md", " \n\t\n")]);
    assert_eq!(strings(leis::plans(&s)), vec!["L6 crates/veredas-abaco: docs/abaco-plan.md is empty"]);
}

#[test]
fn l6_a_plan_holding_only_a_byte_order_mark_is_empty() {
    let abaco = pkg("veredas-abaco", "2.0.0");
    for blank in ["\u{FEFF}", "\u{FEFF}\r\n", "\u{200B}", "\u{FEFF} \u{2060}\n"] {
        let s = suite(&ws(&["crates/veredas-abaco"]), &[("crates/veredas-abaco", &abaco)], &[("docs/abaco-plan.md", blank)]);
        assert_eq!(strings(leis::plans(&s)), vec!["L6 crates/veredas-abaco: docs/abaco-plan.md is empty"], "{blank:?}");
    }
    // and a BOM before real text is a plan
    let s = suite(&ws(&["crates/veredas-abaco"]), &[("crates/veredas-abaco", &abaco)], &[("docs/abaco-plan.md", "\u{FEFF}# Ábaco\n")]);
    assert_eq!(strings(leis::plans(&s)), NONE);
}

#[test]
fn l6_a_plan_with_a_title_passes() {
    let abaco = pkg("veredas-abaco", "2.0.0");
    let s = suite(&ws(&["crates/veredas-abaco"]), &[("crates/veredas-abaco", &abaco)], &[("docs/abaco-plan.md", "# Ábaco\n")]);
    assert_eq!(strings(leis::plans(&s)), NONE);
}

#[test]
fn l6_the_language_crates_share_docs_orcl() {
    let std = pkg("orcl-std", "2.0.0");
    let without = suite(&ws(&["crates/orcl-std"]), &[("crates/orcl-std", &std)], &[]);
    assert_eq!(
        strings(leis::plans(&without)),
        vec!["L6 crates/orcl-std: no docs/orcl/README.md (a role nobody covers, written before the code)"]
    );
    let with = suite(&ws(&["crates/orcl-std"]), &[("crates/orcl-std", &std)], &[("docs/orcl/README.md", "# ORCL\n")]);
    assert_eq!(strings(leis::plans(&with)), NONE);
}

#[test]
fn l6_a_tool_needs_no_plan() {
    let s = suite(&ws(&["tools/leis"]), &[("tools/leis", LEIS)], &[]);
    assert_eq!(strings(leis::plans(&s)), NONE);
}

#[test]
fn l6_a_veredas_crate_under_tools_is_caught_by_the_laws_together() {
    // L6 itself judges components only; L2 names the crate that hides under tools/, so the plan
    // requirement cannot be escaped by the folder
    let m = pkg("veredas-maquinar", "2.0.0");
    let s = suite(&ws(&["tools/veredas-maquinar"]), &[("tools/veredas-maquinar", &m)], &[]);
    assert_eq!(strings(leis::plans(&s)), NONE);
    assert_eq!(
        strings(leis::all(&s)),
        vec!["L2 tools/veredas-maquinar: a veredas-* crate is a component; it lives under crates/"]
    );
}

/// Runs git in a throwaway repository with a fixed identity; panics on failure (a test may).
fn git(root: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["-c", "user.name=leis", "-c", "user.email=leis@veredas", "-c", "core.autocrlf=false"])
        .args(args)
        .output()
        .expect("git runs");
    assert!(out.status.success(), "git {:?}: {}", args, String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(root: &std::path::Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("the folder is made");
    std::fs::write(&path, text).expect("the file is written");
}

fn commit(root: &std::path::Path, message: &str) -> String {
    git(root, &["add", "-A"]);
    git(root, &["commit", "-q", "--allow-empty", "-m", message]);
    git(root, &["rev-parse", "--short=7", "HEAD"])
}

const ABACO: &str = "[package]\nname = \"veredas-abaco\"\nversion = \"2.0.0\"\n";
const PLAN: &str = "# Ábaco\n";

fn repo(name: &str, files: &[(&str, &str)]) -> Tree {
    let tree = Tree::new(name, files);
    git(tree.path(), &["init", "-q", "-b", "main"]);
    tree
}

#[test]
fn l6_history_names_the_commit_that_landed_code_without_its_plan() {
    let workspace = ws(&["crates/veredas-abaco"]);

    // the plan one commit late: the landing commit is named, however good the tree is now
    let late = repo("l6-history-late", &[("Cargo.toml", &workspace), ("crates/veredas-abaco/Cargo.toml", ABACO)]);
    let landed = commit(late.path(), "the code");
    write(late.path(), "docs/abaco-plan.md", PLAN);
    commit(late.path(), "the plan, late");
    assert_eq!(strings(leis::plans(&Suite::read(late.path()).expect("reads"))), NONE, "the tree law is content now");
    assert_eq!(
        strings(leis::history(late.path()).expect("git history reads")),
        vec![format!("L6 crates/veredas-abaco: commit {landed} holds the code and no docs/abaco-plan.md")]
    );

    // the plan in the same commit, as the law wants it
    let together = repo(
        "l6-history-together",
        &[("Cargo.toml", &workspace), ("crates/veredas-abaco/Cargo.toml", ABACO), ("docs/abaco-plan.md", PLAN)],
    );
    commit(together.path(), "the plan and the code");
    assert_eq!(strings(leis::history(together.path()).expect("git history reads")), NONE);

    // a blank plan in the landing commit, filled later: the landing commit is named
    let blank = repo(
        "l6-history-blank",
        &[("Cargo.toml", &workspace), ("crates/veredas-abaco/Cargo.toml", ABACO), ("docs/abaco-plan.md", "\u{FEFF}\n")],
    );
    let landed = commit(blank.path(), "a blank plan");
    write(blank.path(), "docs/abaco-plan.md", PLAN);
    commit(blank.path(), "the plan, filled");
    assert_eq!(
        strings(leis::history(blank.path()).expect("git history reads")),
        vec![format!("L6 crates/veredas-abaco: commit {landed} holds the code and docs/abaco-plan.md blank")]
    );
}

#[test]
fn l6_history_sees_through_merges() {
    let workspace = ws(&["crates/veredas-abaco"]);

    // the code lands without its plan on a branch, the same code lands with its plan on main, the
    // branch is merged: simplified history would follow main alone (refute round 3)
    let hidden = repo("l6-history-merge-hidden", &[("Cargo.toml", &workspace)]);
    commit(hidden.path(), "base");
    git(hidden.path(), &["checkout", "-q", "-b", "side"]);
    write(hidden.path(), "crates/veredas-abaco/Cargo.toml", ABACO);
    let landed = commit(hidden.path(), "the code, on the branch");
    write(hidden.path(), "docs/abaco-plan.md", PLAN);
    commit(hidden.path(), "the plan, on the branch");
    git(hidden.path(), &["checkout", "-q", "main"]);
    write(hidden.path(), "crates/veredas-abaco/Cargo.toml", ABACO);
    write(hidden.path(), "docs/abaco-plan.md", PLAN);
    commit(hidden.path(), "the code with its plan, on main");
    git(hidden.path(), &["merge", "-q", "--no-ff", "-m", "merge side", "side"]);
    assert_eq!(
        strings(leis::history(hidden.path()).expect("git history reads")),
        vec![format!("L6 crates/veredas-abaco: commit {landed} holds the code and no docs/abaco-plan.md")]
    );

    // the code first appears in the merge commit itself (refute round 3: a merge shows no diff)
    let in_merge = repo("l6-history-merge-itself", &[("Cargo.toml", &ws(&[]))]);
    commit(in_merge.path(), "base");
    git(in_merge.path(), &["checkout", "-q", "-b", "side"]);
    write(in_merge.path(), "docs/notes.md", "side\n");
    commit(in_merge.path(), "side");
    git(in_merge.path(), &["checkout", "-q", "main"]);
    write(in_merge.path(), "README.md", "main\n");
    commit(in_merge.path(), "main");
    git(in_merge.path(), &["merge", "-q", "--no-ff", "--no-commit", "side"]);
    write(in_merge.path(), "Cargo.toml", &workspace);
    write(in_merge.path(), "crates/veredas-abaco/Cargo.toml", ABACO);
    let landed = commit(in_merge.path(), "merge, with the code");
    write(in_merge.path(), "docs/abaco-plan.md", PLAN);
    commit(in_merge.path(), "the plan, late");
    assert_eq!(
        strings(leis::history(in_merge.path()).expect("git history reads")),
        vec![format!("L6 crates/veredas-abaco: commit {landed} holds the code and no docs/abaco-plan.md")]
    );
}

#[test]
fn l6_history_judges_every_commit_that_holds_the_code() {
    let workspace = ws(&["crates/veredas-abaco"]);

    // removed and re-added without its plan: the second life is judged too (refute round 3)
    let again = repo(
        "l6-history-readded",
        &[("Cargo.toml", &workspace), ("crates/veredas-abaco/Cargo.toml", ABACO), ("docs/abaco-plan.md", PLAN)],
    );
    commit(again.path(), "the plan and the code");
    git(again.path(), &["rm", "-q", "-r", "crates/veredas-abaco", "docs/abaco-plan.md"]);
    commit(again.path(), "both gone");
    write(again.path(), "crates/veredas-abaco/Cargo.toml", ABACO);
    let landed = commit(again.path(), "the code again, no plan");
    write(again.path(), "docs/abaco-plan.md", PLAN);
    commit(again.path(), "the plan again");
    assert_eq!(
        strings(leis::history(again.path()).expect("git history reads")),
        vec![format!("L6 crates/veredas-abaco: commit {landed} holds the code and no docs/abaco-plan.md")]
    );

    // the plan deleted then restored while the code stayed: the commit without it is named
    let gap = repo(
        "l6-history-plan-gap",
        &[("Cargo.toml", &workspace), ("crates/veredas-abaco/Cargo.toml", ABACO), ("docs/abaco-plan.md", PLAN)],
    );
    commit(gap.path(), "the plan and the code");
    git(gap.path(), &["rm", "-q", "docs/abaco-plan.md"]);
    let without = commit(gap.path(), "the plan gone");
    write(gap.path(), "docs/abaco-plan.md", PLAN);
    commit(gap.path(), "the plan back");
    assert_eq!(
        strings(leis::history(gap.path()).expect("git history reads")),
        vec![format!("L6 crates/veredas-abaco: commit {without} holds the code and no docs/abaco-plan.md")]
    );
}

#[test]
fn l6_history_judges_components_no_longer_at_head() {
    // renamed: the old folder landed without its plan, however complete the new one is
    let renamed = repo(
        "l6-history-renamed",
        &[("Cargo.toml", &ws(&["crates/veredas-velho"])), ("crates/veredas-velho/Cargo.toml", &pkg("veredas-velho", "2.0.0"))],
    );
    let landed = commit(renamed.path(), "velho, no plan");
    git(renamed.path(), &["mv", "crates/veredas-velho", "crates/veredas-novo"]);
    write(renamed.path(), "Cargo.toml", &ws(&["crates/veredas-novo"]));
    write(renamed.path(), "crates/veredas-novo/Cargo.toml", &pkg("veredas-novo", "2.0.0"));
    write(renamed.path(), "docs/novo-plan.md", "# Novo\n");
    commit(renamed.path(), "renamed to novo, with its plan");
    assert_eq!(strings(leis::plans(&Suite::read(renamed.path()).expect("reads"))), NONE);
    assert_eq!(
        strings(leis::history(renamed.path()).expect("git history reads")),
        vec![format!("L6 crates/veredas-velho: commit {landed} holds the code and no docs/velho-plan.md")]
    );

    // deleted: it landed without its plan and is gone now; the landing is still judged
    let deleted = repo(
        "l6-history-deleted",
        &[("Cargo.toml", &ws(&["crates/veredas-abaco"])), ("crates/veredas-abaco/Cargo.toml", ABACO)],
    );
    let landed = commit(deleted.path(), "abaco, no plan");
    git(deleted.path(), &["rm", "-q", "-r", "crates/veredas-abaco"]);
    write(deleted.path(), "Cargo.toml", &ws(&[]));
    commit(deleted.path(), "abaco gone");
    assert_eq!(
        strings(leis::history(deleted.path()).expect("git history reads")),
        vec![format!("L6 crates/veredas-abaco: commit {landed} holds the code and no docs/abaco-plan.md")]
    );
}

#[test]
fn l6_history_wants_the_plan_as_a_regular_file() {
    // a folder where the plan should be is no plan, in a commit as in the tree
    let folder = repo(
        "l6-history-plan-folder",
        &[("Cargo.toml", &ws(&["crates/veredas-abaco"])), ("crates/veredas-abaco/Cargo.toml", ABACO), ("docs/abaco-plan.md/x.txt", " \n")],
    );
    let landed = commit(folder.path(), "a folder for a plan");
    git(folder.path(), &["rm", "-q", "-r", "docs/abaco-plan.md"]);
    write(folder.path(), "docs/abaco-plan.md", PLAN);
    commit(folder.path(), "a real plan");
    assert_eq!(
        strings(leis::history(folder.path()).expect("git history reads")),
        vec![format!("L6 crates/veredas-abaco: commit {landed} holds the code and no docs/abaco-plan.md")]
    );

    // a symlink where the plan should be is no plan either (staged as mode 120000, no filesystem link needed)
    let link = repo(
        "l6-history-plan-link",
        &[("Cargo.toml", &ws(&["crates/veredas-abaco"])), ("crates/veredas-abaco/Cargo.toml", ABACO), ("target.txt", "nowhere.md")],
    );
    let blob = git(link.path(), &["hash-object", "-w", "target.txt"]);
    std::fs::remove_file(link.path().join("target.txt")).expect("the helper file goes");
    git(link.path(), &["add", "-A"]);
    git(link.path(), &["update-index", "--add", "--cacheinfo", &format!("120000,{blob},docs/abaco-plan.md")]);
    git(link.path(), &["commit", "-q", "-m", "a link for a plan"]);
    let landed = git(link.path(), &["rev-parse", "--short=7", "HEAD"]);
    git(link.path(), &["rm", "-q", "--cached", "docs/abaco-plan.md"]);
    write(link.path(), "docs/abaco-plan.md", PLAN);
    commit(link.path(), "a real plan");
    assert_eq!(
        strings(leis::history(link.path()).expect("git history reads")),
        vec![format!("L6 crates/veredas-abaco: commit {landed} holds the code and no docs/abaco-plan.md")]
    );
}

#[test]
fn l6_history_answers_on_a_long_history() {
    // thousands of commits in one go (refute round 4: the git conversation deadlocked past a
    // thousand): built with fast-import, judged within a bound
    let workspace = ws(&["crates/veredas-abaco"]);
    let tree = repo("l6-history-long", &[]);
    let mut stream = String::new();
    for (mark, text) in [(1, workspace.as_str()), (2, ABACO), (3, PLAN)] {
        stream.push_str(&format!("blob\nmark :{mark}\ndata {}\n{text}\n", text.len()));
    }
    stream.push_str(
        "commit refs/heads/main\nmark :4\ncommitter leis <leis@veredas> 1700000000 +0000\ndata 4\nbase\n\
         M 100644 :1 Cargo.toml\nM 100644 :2 crates/veredas-abaco/Cargo.toml\nM 100644 :3 docs/abaco-plan.md\n\n",
    );
    let commits = 3000;
    for n in 0..commits {
        stream.push_str(&format!(
            "commit refs/heads/main\nmark :{}\ncommitter leis <leis@veredas> {} +0000\ndata 5\nempty\nfrom :{}\n\n",
            5 + n,
            1700000001 + n,
            4 + n
        ));
    }
    write(tree.path(), ".stream", &stream);
    let fed = std::process::Command::new("git")
        .arg("-C")
        .arg(tree.path())
        .args(["fast-import", "--quiet"])
        .stdin(std::fs::File::open(tree.path().join(".stream")).expect("the stream opens"))
        .output()
        .expect("git runs");
    assert!(fed.status.success(), "{}", String::from_utf8_lossy(&fed.stderr));
    std::fs::remove_file(tree.path().join(".stream")).expect("the stream goes");
    git(tree.path(), &["reset", "-q", "--hard", "main"]);
    assert_eq!(git(tree.path(), &["rev-list", "--count", "HEAD"]), (commits + 1).to_string());
    let started = std::time::Instant::now();
    assert_eq!(strings(leis::history(tree.path()).expect("git history reads")), NONE);
    assert!(started.elapsed() < std::time::Duration::from_secs(120), "judged in {:?}", started.elapsed());
}

#[test]
fn l6_history_refuses_what_it_cannot_judge() {
    let workspace = ws(&["crates/veredas-abaco"]);

    // a component not yet committed is the tree law's business, not history's
    let fresh = repo("l6-history-fresh", &[("Cargo.toml", &workspace), ("crates/veredas-abaco/Cargo.toml", ABACO)]);
    assert_eq!(strings(leis::history(fresh.path()).expect("git history reads")), NONE);
    assert_eq!(
        strings(leis::plans(&Suite::read(fresh.path()).expect("reads"))),
        vec!["L6 crates/veredas-abaco: no docs/abaco-plan.md (a role nobody covers, written before the code)"]
    );

    // no repository at all is an error, never a pass
    let bare = Tree::new("l6-history-norepo", &[("Cargo.toml", &workspace), ("crates/veredas-abaco/Cargo.toml", ABACO)]);
    assert!(leis::history(bare.path()).is_err());

    // a shallow clone has its history cut: an error, where a full clone names the commit
    let origin = repo("l6-history-origin", &[("Cargo.toml", &workspace), ("crates/veredas-abaco/Cargo.toml", ABACO)]);
    let landed = commit(origin.path(), "the code");
    write(origin.path(), "docs/abaco-plan.md", PLAN);
    commit(origin.path(), "the plan, late");
    let url = format!("file:///{}", origin.path().display().to_string().replace('\\', "/"));
    let shallow = Tree::new("l6-history-shallow", &[]);
    git(shallow.path().parent().expect("temp"), &["clone", "-q", "--depth", "1", &url, &shallow.path().display().to_string()]);
    assert!(leis::history(shallow.path()).is_err(), "a shallow clone is refused");
    let full = Tree::new("l6-history-full", &[]);
    git(full.path().parent().expect("temp"), &["clone", "-q", &url, &full.path().display().to_string()]);
    assert_eq!(
        strings(leis::history(full.path()).expect("git history reads")),
        vec![format!("L6 crates/veredas-abaco: commit {landed} holds the code and no docs/abaco-plan.md")]
    );
}

#[test]
fn l6_the_reader_leaves_the_suite_plan_to_nobody() {
    let m = pkg("veredas-veredas", "2.0.0");
    let tree = Tree::new(
        "l6-suite-plan",
        &[
            ("Cargo.toml", &ws(&["crates/veredas-veredas"])),
            ("crates/veredas-veredas/Cargo.toml", &m),
            ("docs/veredas-plan.md", "# Veredas 2: the plan\n"),
            ("docs/abaco-plan.md", "\u{FEFF}"),
            ("docs/orcl/README.md", "# ORCL\n"),
        ],
    );
    let s = Suite::read(tree.path()).expect("the tree reads");
    assert_eq!(s.plans.keys().collect::<Vec<_>>(), vec!["docs/abaco-plan.md", "docs/orcl/README.md"]);
    assert_eq!(
        strings(leis::plans(&s)),
        vec!["L6 crates/veredas-veredas: no docs/veredas-plan.md (a role nobody covers, written before the code)"]
    );
}
