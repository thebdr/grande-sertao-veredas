//! veredas-supervisao, **Supervisão** on the screen: the one place a person watches and acts on a
//! running plant (`docs/supervisao-plan.md`).
//!
//! It starts a plant's portal sessions in its own process from their project files, binds their
//! live values to the objects of a 2D SVG layout and of dashboards by one set of object IDs
//! (`docs/binding-schema.md`), and offers the plant's writes only through portal's gated surface,
//! nested unchanged under each session's path. Every state, staleness, marker place and panel
//! number is computed here, in Rust; the page (`web/`, built into a committed `dist/`) draws.
//!
//! ```text
//! config   the supervisão file: the address, the sessions
//! binding  the binding file: objects, rules, layout, headline, dashboards; checked against the
//!          project and the layout
//! svg      the layout read as XML
//! state    the vocabulary: a class with a mark and a word
//! feed     a session's objects live
//! panel    the dashboards' panels computed
//! host     the sessions started and served side by side
//! web      the page compiled in, and its source hash
//! ```

pub mod binding;
pub mod config;
pub mod feed;
pub mod host;
pub mod panel;
pub mod state;
pub mod svg;
pub mod web;

pub use config::{HostConfig, SessionSpec};
pub use host::{Host, HostError};
