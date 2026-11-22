//! The Rust side of Oficina's own-shell spike that the bridge seam hides (docs/oficina-plan.md,
//! "The own-shell spike"): what the interface shows is computed here, and nothing here knows Qt,
//! QML or which bridge joins them. The shell drives these types; its tests drive the shell.
//!
//! - [`tree`]: a project folder as the tree the workbench shows, read lazily, one folder at a
//!   time, each TOML file named by its stamp through core's one header reader.
//! - [`document`]: one TOML file as the document behind its two views, the form and the text,
//!   whose edits keep every comment, space, quote and line ending they do not touch.
//! - [`formats`]: which component reads each `veredas-*` format.
//! - [`table`]: a table of a million rows computed cell by cell when a view asks.
//! - [`lsp`]: a language-server client: rust-analyzer on an ORCL file, running nothing of the folder.
//!
//! A spike, not a member: these tests drive no window, so by the plan's rule they prove no clause
//! ("A test that does not drive the real interface proves no clause"). What they find is the
//! spike's evidence (`.zen/evidence/P-015.md`).

pub mod document;
pub mod formats;
pub mod lsp;
pub mod table;
pub mod tree;
