//! L3 (C-003): no shared workspace version; every component states its own SemVer, from 2.0.0.

mod common;

use common::*;

#[test]
fn l3_a_shared_workspace_version_is_named() {
    let workspace = "[workspace]\nmembers = [\"tools/leis\"]\n[workspace.package]\nversion = \"0.2.1\"\nedition = \"2024\"\n";
    let s = suite(workspace, &[("tools/leis", LEIS)], &[]);
    assert_eq!(
        strings(leis::versioning(&s)),
        vec!["L3 Cargo.toml: [workspace.package] carries a version; each component owns its own"]
    );
}

#[test]
fn l3_a_component_inheriting_its_version_is_named() {
    let m = "[package]\nname = \"veredas-portal\"\nversion.workspace = true\n";
    let s = suite(&ws(&["crates/veredas-portal"]), &[("crates/veredas-portal", m)], &[]);
    assert_eq!(
        strings(leis::versioning(&s)),
        vec!["L3 crates/veredas-portal: version.workspace = true; a component states its own version"]
    );
}

#[test]
fn l3_a_version_below_2_is_named() {
    let m = pkg("veredas-portal", "1.9.0");
    let s = suite(&ws(&["crates/veredas-portal"]), &[("crates/veredas-portal", &m)], &[]);
    assert_eq!(
        strings(leis::versioning(&s)),
        vec!["L3 crates/veredas-portal: version 1.9.0 is below 2.0.0 (every component starts at 2.0.0)"]
    );
}

#[test]
fn l3_a_version_that_is_not_semver_is_named() {
    let m = pkg("veredas-portal", "two");
    let s = suite(&ws(&["crates/veredas-portal"]), &[("crates/veredas-portal", &m)], &[]);
    assert_eq!(strings(leis::versioning(&s)), vec!["L3 crates/veredas-portal: version `two` is not SemVer"]);
}

#[test]
fn l3_a_component_without_a_version_is_named() {
    let m = "[package]\nname = \"veredas-portal\"\n";
    let s = suite(&ws(&["crates/veredas-portal"]), &[("crates/veredas-portal", m)], &[]);
    assert_eq!(strings(leis::versioning(&s)), vec!["L3 crates/veredas-portal: no version"]);
}

#[test]
fn l3_a_prerelease_from_2_passes_and_a_tool_is_free() {
    let m = pkg("veredas-portal", "2.0.0-alpha.1");
    let s = suite(&ws(&["crates/veredas-portal", "tools/leis"]), &[("crates/veredas-portal", &m), ("tools/leis", LEIS)], &[]);
    assert_eq!(strings(leis::versioning(&s)), NONE);
}
