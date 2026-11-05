//! The real tree, read from disk, breaks no law. Linked by every clause: the laws hold on the
//! suite as it is, not only on the synthetic suites that prove them.

mod common;

use std::path::Path;

use common::*;
use leis::Suite;

#[test]
fn the_real_tree_obeys_every_law() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    let s = Suite::read(&root).expect("the repo reads");
    assert!(s.found.contains(&"tools/leis".to_string()), "the reader found this crate: {:?}", s.found);
    assert!(s.members.iter().any(|m| m.name == "leis"), "and it is a member");
    assert!(!s.workspace_version, "the workspace carries no shared version");
    assert!(!s.plans.contains_key("docs/veredas-plan.md"), "the suite plan is nobody's");
    assert_eq!(strings(leis::all(&s)), NONE);
    assert_eq!(strings(leis::history(&root).expect("the repo's history reads")), NONE);
}
