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
