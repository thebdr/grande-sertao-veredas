//! L2 (C-002): a component is `veredas-<nome>` in a folder of its own name; `orcl-*` keep their
//! names; a `veredas-*` crate is a component and lives under `crates/`.

mod common;

use common::*;

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

#[test]
fn l2_the_nome_is_a_letter_then_letters_and_digits() {
    let refused = ["veredas-abaco-py", "veredas-abaco_py", "veredas--x", "veredas-1abc", "veredas-abaco "];
    for name in refused {
        let m = pkg(name, "2.0.0");
        let dir = format!("crates/{name}");
        let s = suite(&ws(&[&dir]), &[(&dir, &m)], &[]);
        assert_eq!(
            strings(leis::naming(&s)),
            vec![format!("L2 {dir}: a component is named veredas-<nome>, <nome> lowercase ASCII (got `{name}`)")],
            "{name:?}"
        );
    }
    let fine = pkg("veredas-core2", "2.0.0");
    let s = suite(&ws(&["crates/veredas-core2"]), &[("crates/veredas-core2", &fine)], &[]);
    assert_eq!(strings(leis::naming(&s)), NONE);
}

#[test]
fn l2_the_folder_matches_the_name_exactly() {
    // the filesystem may ignore case; the law does not
    let m = pkg("veredas-abaco", "2.0.0");
    let s = suite(&ws(&["crates/Veredas-Abaco"]), &[("crates/Veredas-Abaco", &m)], &[]);
    assert_eq!(strings(leis::naming(&s)), vec!["L2 crates/Veredas-Abaco: the folder is named after the crate (veredas-abaco)"]);
}

#[test]
fn l2_the_language_crates_follow_the_same_nome_rule() {
    for name in ["orcl-", "orcl-Á", "orcl-Portal", "orcl-std_x"] {
        let m = pkg(name, "2.0.0");
        let dir = format!("crates/{name}");
        let s = suite(&ws(&[&dir]), &[(&dir, &m)], &[]);
        assert_eq!(
            strings(leis::naming(&s)),
            vec![format!("L2 {dir}: the language's crates are orcl-<nome>, <nome> lowercase ASCII (got `{name}`)")],
            "{name:?}"
        );
    }
}

#[test]
fn l2_an_extension_inside_its_host_is_not_a_member() {
    // acerto lives inside abaco; listing it as a member breaks L1 twice and L2 once
    let host = pkg("veredas-abaco", "2.0.0");
    let ext = pkg("acerto", "2.0.0");
    let s = suite(
        &ws(&["crates/veredas-abaco", "crates/veredas-abaco/acerto"]),
        &[("crates/veredas-abaco", &host), ("crates/veredas-abaco/acerto", &ext)],
        &[("docs/abaco-plan.md", "# Ábaco\n")],
    );
    assert_eq!(
        strings(leis::all(&s)),
        vec![
            "L1 crates/veredas-abaco/acerto: a member lives directly under crates/ or tools/",
            "L1 crates/veredas-abaco/acerto: a crate lives directly under crates/ or tools/ (a nested folder is not a root; exclude what is not the suite's)",
            "L2 crates/veredas-abaco/acerto: a component is named veredas-<nome>, <nome> lowercase ASCII (got `acerto`)",
        ]
    );
}

#[test]
fn l2_a_veredas_crate_under_tools_is_named() {
    let m = pkg("veredas-maquinar", "2.0.0");
    let s = suite(&ws(&["tools/veredas-maquinar", "tools/leis"]), &[("tools/veredas-maquinar", &m), ("tools/leis", LEIS)], &[]);
    assert_eq!(
        strings(leis::naming(&s)),
        vec!["L2 tools/veredas-maquinar: a veredas-* crate is a component; it lives under crates/"]
    );
}
