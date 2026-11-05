//! L5 (C-005): every component's normal and build edges to other members are in the map.

mod common;

use common::*;
use leis::Suite;

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
fn l5_a_dependency_renamed_in_the_workspace_table_is_seen() {
    let workspace = "[workspace]\nmembers = [\"crates/veredas-core\", \"crates/veredas-portal\", \"crates/veredas-maquinar\"]\n\
                     [workspace.package]\nedition = \"2024\"\n\
                     [workspace.dependencies]\ngateway = { package = \"veredas-portal\", path = \"crates/veredas-portal\" }\n";
    let maquinar = "[package]\nname = \"veredas-maquinar\"\nversion = \"2.0.0\"\n\
                    [dependencies]\ngateway = { workspace = true }\n";
    let core = pkg("veredas-core", "2.0.0");
    let portal = with_deps("veredas-portal", &["veredas-core"]);
    let s = suite(
        workspace,
        &[("crates/veredas-core", &core), ("crates/veredas-portal", &portal), ("crates/veredas-maquinar", maquinar)],
        &[],
    );
    assert_eq!(
        strings(leis::direction(&s)),
        vec!["L5 crates/veredas-maquinar: depends on veredas-portal in [dependencies]; the map allows veredas-maquinar → veredas-core, veredas-aranha"]
    );
}

/// The clause's own sentence, copied here on purpose and not read from `MAP`: what each component
/// may use besides core. `orcl-*` is the family, as the clause says it.
const CLAUSE: &[(&str, &[&str])] = &[
    ("veredas-core", &[]),
    ("veredas-portal", &[]),
    ("veredas-maquinar", &["veredas-aranha"]),
    ("veredas-aranha", &[]),
    ("veredas-controlador", &["veredas-portal", "veredas-aranha", "orcl-*"]),
    ("veredas-comissionador", &["veredas-portal"]),
    ("veredas-supervisao", &["veredas-portal"]),
    ("veredas-abaco", &["veredas-portal"]),
    ("veredas-wms", &["veredas-portal"]),
    ("orcl-std", &["orcl-*"]),
    ("orcl-runtime", &["orcl-*"]),
];

fn clause_allows(pattern: &str, name: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => name.starts_with(prefix),
        None => pattern == name,
    }
}

#[test]
fn l5_every_row_of_the_clause_is_pinned() {
    // every (component, member) pair of the planned suite, judged against the clause's copy of the
    // map: a row that drifts in MAP, wider or narrower, fails here
    let names: Vec<&str> = CLAUSE.iter().map(|(n, _)| *n).collect();
    let mut pairs = 0;
    for (a, row) in CLAUSE {
        for b in &names {
            if b == a {
                continue;
            }
            let crates: Vec<(String, String)> = names
                .iter()
                .map(|n| (format!("crates/{n}"), if n == a { with_deps(a, &[b]) } else { pkg(n, "2.0.0") }))
                .collect();
            let refs: Vec<(&str, &str)> = crates.iter().map(|(d, t)| (d.as_str(), t.as_str())).collect();
            let dirs: Vec<&str> = crates.iter().map(|(d, _)| d.as_str()).collect();
            let s = suite(&ws(&dirs), &refs, &[]);
            let got = strings(leis::direction(&s));
            let allowed = *b == "veredas-core" || row.iter().any(|p| clause_allows(p, b));
            if allowed {
                assert_eq!(got, NONE, "{a} → {b} is in the clause");
            } else {
                let mut listed = vec!["veredas-core"];
                listed.extend(row.iter().copied());
                assert_eq!(
                    got,
                    vec![format!(
                        "L5 crates/{a}: depends on {b} in [dependencies]; the map allows {a} → {}",
                        listed.join(", ")
                    )],
                    "{a} → {b} is not in the clause"
                );
            }
            pairs += 1;
        }
    }
    assert_eq!(pairs, 110);
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
