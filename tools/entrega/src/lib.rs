//! entrega: what Veredas 2 ships (docs/entrega-plan.md). The suite release's manifest and its
//! check against the repository's history; the cut, which builds a release's executables in a
//! builder machine of the tester machine's distribution, packs the bundle and runs it together;
//! and the virtual machines both run in. Repository tooling, not a component.

pub mod builder;
pub mod bundle;
pub mod cache;
pub mod cut;
pub mod git;
pub mod machine;
pub mod manifest;
pub mod maquina;
pub mod together;
pub mod vm;
