//! Oficina, the workshop of Veredas 2 (docs/oficina-plan.md): the one native application through
//! which a person sets up, programs, monitors and commissions every component.
//!
//! What the interface shows is computed here, in Rust, with no Qt: the model builds on any machine
//! the suite builds on.
//! - [`tree`]: a project folder as the tree the workbench shows, read lazily, one folder at a time,
//!   each TOML file named by its stamp through core's one header reader.
//! - [`formats`]: which component reads each `veredas-*` format.
//!
//! The window is behind the `shell` feature: Qt Quick, its logic in Rust through the cxx-qt bridge,
//! and the one seam between them, a private module nothing else names ("One seam for the bridge").
//! [`run`] starts it.

pub mod formats;
pub mod tree;

#[cfg(feature = "shell")]
mod shell;

#[cfg(feature = "shell")]
pub use shell::run;
