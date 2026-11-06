//! veredas-maquinar: the bench of Veredas 2.
//!
//! Simulated hardware on the real wire, by configuration, so every consumer meets its plant on a
//! bench before it meets the plant (`docs/maquinar-plan.md`). A test tool that owns its sockets
//! and never runs in production; a library first, with one executable on top.
//!
//! - [`emit`]: log emitters, writing rotating files the way a plant logger writes them.
//!
//! Nothing here names a device, a vendor or a product: what a controller's telegrams mean is
//! configuration (a channel file, a catalog, a rules file), as in core (`docs/questions.md` Q8).

pub mod emit;
