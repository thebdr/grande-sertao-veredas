//! The laws' verdict. Each law is attacked on synthetic suites (exact violations, by message) and
//! then the real tree is read and must break none of them. The clause of each law in
//! `.zen/contract.md` names its tests here.

use std::collections::BTreeMap;
use std::path::Path;

use leis::{Suite, Violation};

fn ws(members: &[&str]) -> String {
    let list: Vec<String> = members.iter().map(|m| format!("\"{m}\"")).collect();
    format!("[workspace]\nmembers = [{}]\n[workspace.package]\nedition = \"2024\"\n", list.join(", "))
}

fn pkg(name: &str, version: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\n")
}

fn suite(workspace: &str, crates: &[(&str, &str)], plans: &[(&str, &str)]) -> Suite {
    let plans: BTreeMap<String, String> = plans.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    Suite::parse(workspace, crates, plans).expect("the synthetic suite parses")
}

fn strings(v: Vec<Violation>) -> Vec<String> {
    v.iter().map(ToString::to_string).collect()
}

const NONE: Vec<String> = Vec::new();
const LEIS: &str = "[package]\nname = \"leis\"\nversion = \"0.1.0\"\n";

// ---- L1 roots -------------------------------------------------------------------------------

#[test]
fn l1_a_member_outside_the_roots_is_named() {
    let s = suite(&ws(&["demos/conveyor"]), &[], &[]);
    assert_eq!(strings(leis::roots(&s)), vec!["L1 demos/conveyor: a member lives under crates/ or tools/"]);
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
fn l1_a_nested_folder_is_not_a_root() {
    let deep = pkg("veredas-abaco", "2.0.0");
    let s = suite(&ws(&["crates/abaco/veredas-abaco"]), &[("crates/abaco/veredas-abaco", &deep)], &[]);
    // listed, so the "not listed" rule stays quiet: one violation, the one that names the cause
    assert_eq!(
        strings(leis::roots(&s)),
        vec!["L1 crates/abaco/veredas-abaco: a member lives under crates/ or tools/"]
    );
}

// ---- L2 naming ------------------------------------------------------------------------------

#[test]
fn l2_a_component_without_the_prefix_is_named() {
    let m = pkg("sorter", "2.0.0");
    let s = suite(&ws(&["crates/sorter"]), &[("crates/sorter", &m)], &[]);
    assert_eq!(
        strings(leis::naming(&s)),
        vec!["L2 crates/sorter: a component is named veredas-<nome>, <nome> lowercase ASCII (got `sorter`)"]
    );
}

#[test]
fn l2_an_uppercase_or_accented_nome_is_named() {
    let upper = pkg("veredas-Abaco", "2.0.0");
    let accented = pkg("veredas-ábaco", "2.0.0");
    let s = suite(
        &ws(&["crates/veredas-Abaco", "crates/veredas-ábaco"]),
        &[("crates/veredas-Abaco", &upper), ("crates/veredas-ábaco", &accented)],
        &[],
    );
    assert_eq!(
        strings(leis::naming(&s)),
        vec![
            "L2 crates/veredas-Abaco: a component is named veredas-<nome>, <nome> lowercase ASCII (got `veredas-Abaco`)",
            "L2 crates/veredas-ábaco: a component is named veredas-<nome>, <nome> lowercase ASCII (got `veredas-ábaco`)",
        ]
    );
}

#[test]
fn l2_a_bare_prefix_is_named() {
    let bare = pkg("veredas-", "2.0.0");
    let s = suite(&ws(&["crates/veredas-"]), &[("crates/veredas-", &bare)], &[]);
    assert_eq!(
        strings(leis::naming(&s)),
        vec!["L2 crates/veredas-: a component is named veredas-<nome>, <nome> lowercase ASCII (got `veredas-`)"]
    );
}

#[test]
fn l2_a_folder_not_named_after_its_crate_is_named() {
    let m = pkg("veredas-abaco", "2.0.0");
    let s = suite(&ws(&["crates/abaco"]), &[("crates/abaco", &m)], &[]);
    assert_eq!(strings(leis::naming(&s)), vec!["L2 crates/abaco: the folder is named after the crate (veredas-abaco)"]);
}

#[test]
fn l2_the_language_crates_keep_their_names() {
    let std = pkg("orcl-std", "2.0.0");
    let runtime = pkg("orcl-runtime", "2.0.0");
    let s = suite(
        &ws(&["crates/orcl-std", "crates/orcl-runtime"]),
        &[("crates/orcl-std", &std), ("crates/orcl-runtime", &runtime)],
        &[],
    );
    assert_eq!(strings(leis::naming(&s)), NONE);
}

#[test]
fn l2_a_tool_is_not_a_component() {
    let s = suite(&ws(&["tools/leis"]), &[("tools/leis", LEIS)], &[]);
    assert_eq!(strings(leis::naming(&s)), NONE);
}

// ---- L3 versioning --------------------------------------------------------------------------

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

// ---- L4 core --------------------------------------------------------------------------------

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

// ---- L5 direction ---------------------------------------------------------------------------

fn with_deps(name: &str, deps: &[&str]) -> String {
    let mut m = pkg(name, "2.0.0");
    m.push_str("[dependencies]\n");
    for d in deps {
        m.push_str(&format!("{d} = {{ path = \"../{d}\" }}\n"));
    }
    m
}

/// The eight components, the language and the tooling, wired as the plan draws them.
fn planned() -> Suite {
    let core = pkg("veredas-core", "2.0.0");
    let portal = with_deps("veredas-portal", &["veredas-core"]);
    let controlador = with_deps("veredas-controlador", &["veredas-core", "veredas-portal", "veredas-aranha", "orcl-std", "orcl-runtime"]);
    let maquinar = with_deps("veredas-maquinar", &["veredas-core", "veredas-aranha"]);
    let aranha = with_deps("veredas-aranha", &["veredas-core"]);
    let comissionador = with_deps("veredas-comissionador", &["veredas-core", "veredas-portal"]);
    let abaco = with_deps("veredas-abaco", &["veredas-core", "veredas-portal"]);
    let supervisao = with_deps("veredas-supervisao", &["veredas-core", "veredas-portal"]);
    let wms = with_deps("veredas-wms", &["veredas-core", "veredas-portal"]);
    let orcl_std = pkg("orcl-std", "2.0.0");
    let orcl_runtime = with_deps("orcl-runtime", &["orcl-std"]);
    let dirs = [
        "crates/veredas-core", "crates/veredas-portal", "crates/veredas-controlador", "crates/veredas-maquinar",
        "crates/veredas-aranha", "crates/veredas-comissionador", "crates/veredas-abaco", "crates/veredas-supervisao",
        "crates/veredas-wms", "crates/orcl-std", "crates/orcl-runtime", "tools/leis",
    ];
    let plan = "# a plan\n";
    suite(
        &ws(&dirs),
        &[
            ("crates/veredas-core", &core), ("crates/veredas-portal", &portal), ("crates/veredas-controlador", &controlador),
            ("crates/veredas-maquinar", &maquinar), ("crates/veredas-aranha", &aranha),
            ("crates/veredas-comissionador", &comissionador), ("crates/veredas-abaco", &abaco),
            ("crates/veredas-supervisao", &supervisao), ("crates/veredas-wms", &wms),
            ("crates/orcl-std", &orcl_std), ("crates/orcl-runtime", &orcl_runtime), ("tools/leis", LEIS),
        ],
        &[
            ("docs/core-plan.md", plan), ("docs/portal-plan.md", plan), ("docs/controlador-plan.md", plan),
            ("docs/maquinar-plan.md", plan), ("docs/aranha-plan.md", plan), ("docs/comissionador-plan.md", plan),
            ("docs/abaco-plan.md", plan), ("docs/supervisao-plan.md", plan), ("docs/wms-plan.md", plan),
            ("docs/orcl/README.md", plan),
        ],
    )
}

#[test]
fn l5_the_planned_suite_breaks_no_law() {
    let s = planned();
    assert_eq!(s.members.len(), 12);
    assert_eq!(strings(leis::all(&s)), NONE);
}

#[test]
fn l5_an_app_depending_on_another_app_is_named() {
    let supervisao = with_deps("veredas-supervisao", &["veredas-core", "veredas-portal", "veredas-comissionador"]);
    let comissionador = with_deps("veredas-comissionador", &["veredas-core"]);
    let core = pkg("veredas-core", "2.0.0");
    let portal = with_deps("veredas-portal", &["veredas-core"]);
    let s = suite(
        &ws(&["crates/veredas-core", "crates/veredas-portal", "crates/veredas-supervisao", "crates/veredas-comissionador"]),
        &[
            ("crates/veredas-core", &core), ("crates/veredas-portal", &portal),
            ("crates/veredas-supervisao", &supervisao), ("crates/veredas-comissionador", &comissionador),
        ],
        &[],
    );
    assert_eq!(
        strings(leis::direction(&s)),
        vec!["L5 crates/veredas-supervisao: depends on veredas-comissionador in [dependencies]; the map allows veredas-supervisao → veredas-core, veredas-portal"]
    );
}

#[test]
fn l5_the_bench_never_pulls_portal() {
    let maquinar = with_deps("veredas-maquinar", &["veredas-core", "veredas-portal"]);
    let core = pkg("veredas-core", "2.0.0");
    let portal = with_deps("veredas-portal", &["veredas-core"]);
    let s = suite(
        &ws(&["crates/veredas-core", "crates/veredas-portal", "crates/veredas-maquinar"]),
        &[("crates/veredas-core", &core), ("crates/veredas-portal", &portal), ("crates/veredas-maquinar", &maquinar)],
        &[],
    );
    assert_eq!(
        strings(leis::direction(&s)),
        vec!["L5 crates/veredas-maquinar: depends on veredas-portal in [dependencies]; the map allows veredas-maquinar → veredas-core, veredas-aranha"]
    );
}

#[test]
fn l5_portal_never_depends_on_an_app() {
    let portal = with_deps("veredas-portal", &["veredas-core", "veredas-supervisao"]);
    let core = pkg("veredas-core", "2.0.0");
    let supervisao = with_deps("veredas-supervisao", &["veredas-core"]);
    let s = suite(
        &ws(&["crates/veredas-core", "crates/veredas-portal", "crates/veredas-supervisao"]),
        &[("crates/veredas-core", &core), ("crates/veredas-portal", &portal), ("crates/veredas-supervisao", &supervisao)],
        &[],
    );
    assert_eq!(
        strings(leis::direction(&s)),
        vec!["L5 crates/veredas-portal: depends on veredas-supervisao in [dependencies]; the map allows veredas-portal → veredas-core"]
    );
}

#[test]
fn l5_a_component_outside_the_map_may_use_core_alone() {
    let novo_ok = with_deps("veredas-novo", &["veredas-core"]);
    let novo_bad = with_deps("veredas-outro", &["veredas-core", "veredas-portal"]);
    let core = pkg("veredas-core", "2.0.0");
    let portal = with_deps("veredas-portal", &["veredas-core"]);
    let s = suite(
        &ws(&["crates/veredas-core", "crates/veredas-portal", "crates/veredas-novo", "crates/veredas-outro"]),
        &[
            ("crates/veredas-core", &core), ("crates/veredas-portal", &portal),
            ("crates/veredas-novo", &novo_ok), ("crates/veredas-outro", &novo_bad),
        ],
        &[],
    );
    assert_eq!(
        strings(leis::direction(&s)),
        vec!["L5 crates/veredas-outro: depends on veredas-portal in [dependencies]; the map allows veredas-outro → veredas-core"]
    );
}

#[test]
fn l5_a_build_dependency_counts_and_a_dev_dependency_is_free() {
    let supervisao = "[package]\nname = \"veredas-supervisao\"\nversion = \"2.0.0\"\n\
                      [dev-dependencies]\nveredas-maquinar = { path = \"../veredas-maquinar\" }\n\
                      [build-dependencies]\nveredas-abaco = { path = \"../veredas-abaco\" }\n";
    let maquinar = pkg("veredas-maquinar", "2.0.0");
    let abaco = pkg("veredas-abaco", "2.0.0");
    let s = suite(
        &ws(&["crates/veredas-supervisao", "crates/veredas-maquinar", "crates/veredas-abaco"]),
        &[("crates/veredas-supervisao", supervisao), ("crates/veredas-maquinar", &maquinar), ("crates/veredas-abaco", &abaco)],
        &[],
    );
    assert_eq!(
        strings(leis::direction(&s)),
        vec!["L5 crates/veredas-supervisao: depends on veredas-abaco in [build-dependencies]; the map allows veredas-supervisao → veredas-core, veredas-portal"]
    );
}

#[test]
fn l5_a_tool_may_depend_on_anything() {
    let oracle = "[package]\nname = \"abaco-oracle\"\nversion = \"0.1.0\"\n\
                  [dependencies]\nveredas-abaco = { path = \"../../crates/veredas-abaco\" }\nveredas-portal = { path = \"../../crates/veredas-portal\" }\n";
    let abaco = pkg("veredas-abaco", "2.0.0");
    let portal = pkg("veredas-portal", "2.0.0");
    let s = suite(
        &ws(&["crates/veredas-abaco", "crates/veredas-portal", "tools/abaco-oracle"]),
        &[("crates/veredas-abaco", &abaco), ("crates/veredas-portal", &portal), ("tools/abaco-oracle", oracle)],
        &[],
    );
    assert_eq!(strings(leis::direction(&s)), NONE);
}

#[test]
fn l5_an_external_crate_of_a_member_name_shape_is_not_an_edge() {
    let portal = with_deps("veredas-portal", &["veredas-core", "veredas-telemetry"]);
    let core = pkg("veredas-core", "2.0.0");
    let s = suite(
        &ws(&["crates/veredas-core", "crates/veredas-portal"]),
        &[("crates/veredas-core", &core), ("crates/veredas-portal", &portal)],
        &[],
    );
    assert_eq!(strings(leis::direction(&s)), NONE, "a crate that is not a member is not the map's business");
}

// ---- L6 plan --------------------------------------------------------------------------------

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

// ---- the real tree --------------------------------------------------------------------------

#[test]
fn the_real_tree_obeys_every_law() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let s = Suite::read(&root).expect("the repo reads");
    assert!(s.members.iter().any(|m| m.name == "leis"), "the walker found this crate: {:?}", s.found);
    assert!(!s.workspace_version, "the workspace carries no shared version");
    assert_eq!(strings(leis::all(&s)), NONE);
}
