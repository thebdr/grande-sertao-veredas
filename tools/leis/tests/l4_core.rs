//! L4 (C-004): `veredas-core` depends on no other member, in any dependency table.

mod common;

use common::*;

#[test]
fn l4_core_depending_on_a_member_is_named_in_every_table() {
    let core = "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\n\
                [dependencies]\nveredas-portal = { path = \"../veredas-portal\" }\nserde = \"1\"\n\
                [dev-dependencies]\nveredas-maquinar = { path = \"../veredas-maquinar\" }\n\
                [build-dependencies]\nleis = { path = \"../../tools/leis\" }\n";
    let portal = pkg("veredas-portal", "2.0.0");
    let maquinar = pkg("veredas-maquinar", "2.0.0");
    let s = suite(
        &ws(&["crates/veredas-core", "crates/veredas-portal", "crates/veredas-maquinar", "tools/leis"]),
        &[
            ("crates/veredas-core", core),
            ("crates/veredas-portal", &portal),
            ("crates/veredas-maquinar", &maquinar),
            ("tools/leis", LEIS),
        ],
        &[],
    );
    assert_eq!(
        strings(leis::core(&s)),
        vec![
            "L4 crates/veredas-core: depends on veredas-portal in [dependencies]; core depends on no other member",
            "L4 crates/veredas-core: depends on veredas-maquinar in [dev-dependencies]; core depends on no other member",
            "L4 crates/veredas-core: depends on leis in [build-dependencies]; core depends on no other member",
        ]
    );
}

#[test]
fn l4_core_may_depend_on_itself_for_its_tests() {
    let core = "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\n\
                [features]\ntestkit = []\n\
                [dev-dependencies]\nveredas-core = { path = \".\", features = [\"testkit\"] }\n";
    let s = suite(&ws(&["crates/veredas-core"]), &[("crates/veredas-core", core)], &[]);
    assert_eq!(strings(leis::core(&s)), NONE);
}

#[test]
fn l4_a_target_specific_table_counts() {
    let core = "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\n\
                [target.'cfg(windows)'.dependencies]\nveredas-portal = { path = \"../veredas-portal\" }\n";
    let portal = pkg("veredas-portal", "2.0.0");
    let s = suite(
        &ws(&["crates/veredas-core", "crates/veredas-portal"]),
        &[("crates/veredas-core", core), ("crates/veredas-portal", &portal)],
        &[],
    );
    assert_eq!(
        strings(leis::core(&s)),
        vec!["L4 crates/veredas-core: depends on veredas-portal in [dependencies]; core depends on no other member"]
    );
}

#[test]
fn l4_a_renamed_dependency_is_seen_by_its_package() {
    let core = "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\n\
                [dependencies]\ngateway = { package = \"veredas-portal\", path = \"../veredas-portal\" }\n";
    let portal = pkg("veredas-portal", "2.0.0");
    let s = suite(
        &ws(&["crates/veredas-core", "crates/veredas-portal"]),
        &[("crates/veredas-core", core), ("crates/veredas-portal", &portal)],
        &[],
    );
    assert_eq!(
        strings(leis::core(&s)),
        vec!["L4 crates/veredas-core: depends on veredas-portal in [dependencies]; core depends on no other member"]
    );
}

#[test]
fn l4_a_dependency_renamed_in_the_workspace_table_is_seen() {
    // the root renames it, the member inherits it under a key that is no member's name
    let workspace = "[workspace]\nmembers = [\"crates/veredas-core\", \"crates/veredas-portal\"]\n\
                     [workspace.package]\nedition = \"2024\"\n\
                     [workspace.dependencies]\ngw = { package = \"veredas-portal\", path = \"crates/veredas-portal\" }\n";
    let core = "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\n\
                [dependencies]\ngw = { workspace = true }\n";
    let portal = pkg("veredas-portal", "2.0.0");
    let s = suite(workspace, &[("crates/veredas-core", core), ("crates/veredas-portal", &portal)], &[]);
    assert_eq!(
        strings(leis::core(&s)),
        vec!["L4 crates/veredas-core: depends on veredas-portal in [dependencies]; core depends on no other member"]
    );
    // a `package` written beside `workspace = true` is ignored, as cargo ignores it (refute round 2:
    // cargo tree still showed core → portal): the root entry names the dependency
    let core = "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\n\
                [dependencies]\ngw = { workspace = true, package = \"serde\" }\n";
    let s = suite(workspace, &[("crates/veredas-core", core), ("crates/veredas-portal", &portal)], &[]);
    assert_eq!(
        strings(leis::core(&s)),
        vec!["L4 crates/veredas-core: depends on veredas-portal in [dependencies]; core depends on no other member"]
    );
    // and the evasion of one key in the member, with no rename at the root
    let workspace = "[workspace]\nmembers = [\"crates/veredas-core\", \"crates/veredas-portal\"]\n\
                     [workspace.package]\nedition = \"2024\"\n\
                     [workspace.dependencies]\nveredas-portal = { path = \"crates/veredas-portal\" }\n";
    let core = "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\n\
                [dependencies]\nveredas-portal = { workspace = true, package = \"anything\" }\n";
    let s = suite(workspace, &[("crates/veredas-core", core), ("crates/veredas-portal", &portal)], &[]);
    assert_eq!(
        strings(leis::core(&s)),
        vec!["L4 crates/veredas-core: depends on veredas-portal in [dependencies]; core depends on no other member"]
    );
}

#[test]
fn l4_the_underscore_tables_of_edition_2021_count() {
    let core = "[package]\nname = \"veredas-core\"\nversion = \"2.0.0\"\nedition = \"2021\"\n\
                [dev_dependencies]\nveredas-maquinar = { path = \"../veredas-maquinar\" }\n\
                [build_dependencies]\nveredas-portal = { path = \"../veredas-portal\" }\n\
                [target.'cfg(unix)'.dev_dependencies]\nveredas-aranha = { path = \"../veredas-aranha\" }\n";
    let portal = pkg("veredas-portal", "2.0.0");
    let maquinar = pkg("veredas-maquinar", "2.0.0");
    let aranha = pkg("veredas-aranha", "2.0.0");
    let s = suite(
        &ws(&["crates/veredas-core", "crates/veredas-portal", "crates/veredas-maquinar", "crates/veredas-aranha"]),
        &[
            ("crates/veredas-core", core),
            ("crates/veredas-portal", &portal),
            ("crates/veredas-maquinar", &maquinar),
            ("crates/veredas-aranha", &aranha),
        ],
        &[],
    );
    assert_eq!(
        strings(leis::core(&s)),
        vec![
            "L4 crates/veredas-core: depends on veredas-maquinar in [dev-dependencies]; core depends on no other member",
            "L4 crates/veredas-core: depends on veredas-portal in [build-dependencies]; core depends on no other member",
            "L4 crates/veredas-core: depends on veredas-aranha in [dev-dependencies]; core depends on no other member",
        ]
    );
}
