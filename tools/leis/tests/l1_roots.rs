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
            "L1 crates/abaco/veredas-abaco: a crate lives directly under crates/ or tools/ (a nested folder is not a root; exclude what is not the suite's)",
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
        vec!["crates", "crates/abaco/veredas-abaco", "tools/leis", "tools/leis-extra/inner"],
        "every manifest under a root at any depth, none under target/, node_modules/ or a hidden folder"
    );
    assert_eq!(
        strings(leis::roots(&s)),
        vec![
            "L1 crates: a crate lives directly under crates/ or tools/ (a nested folder is not a root; exclude what is not the suite's)",
            "L1 crates/abaco/veredas-abaco: a crate lives directly under crates/ or tools/ (a nested folder is not a root; exclude what is not the suite's)",
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
