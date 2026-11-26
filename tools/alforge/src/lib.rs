//! alforge: what Veredas 2 ships (docs/alforge-plan.md). The suite release's manifest and its
//! check against the repository's history; the cut, which builds a release's executables in a
//! builder machine of the tester machine's distribution, packs the bundle and runs it together;
//! and the virtual machines both run in. Repository tooling, not a component.

pub mod builder;
pub mod bundle;
pub mod cache;
pub mod components;
pub mod cut;
pub mod git;
pub mod inputs;
pub mod licences;
pub mod machine;
pub mod manifest;
pub mod maquina;
pub mod together;
pub mod trace;
pub mod vm;

use std::path::Path;

/// The member's folder, `tools/alforge`, then the one it had before the architect named it
/// (`tools/entrega`, until 2026-11-25; docs/questions.md Q430), so a release cut before the rename
/// is still built and proven from its own commit's recipe, guide and plant.
pub const FOLDERS: [&str; 2] = ["tools/alforge", "tools/entrega"];

/// The first of [`FOLDERS`] whose recipe (`maquina/maquina.toml`) `commit` holds, as git stores it.
pub fn folder_at(root: &Path, commit: &str) -> Result<Option<&'static str>, git::GitError> {
    for f in FOLDERS {
        if git::object(root, commit, &format!("{f}/maquina/maquina.toml"))?.is_some() {
            return Ok(Some(f));
        }
    }
    Ok(None)
}
