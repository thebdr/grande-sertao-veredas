//! veredas-maquinar: the bench of Veredas 2.
//!
//! Simulated hardware on the real wire, by configuration, so every consumer meets its plant on a
//! bench before it meets the plant (`docs/maquinar-plan.md`). A test tool that owns its sockets
//! and never runs in production; a library first, with one executable on top.
//!
//! - [`emit`]: log emitters, writing rotating files the way a plant logger writes them.
//! - [`opc`]: a configurable OPC UA server on a real endpoint, with reactions and a relay.
//! - [`channel`]: a telegram channel held by core's channel file, speaking a catalog, with
//!   cyclic and on-demand messages and a monitor.
//! - [`controller`]: the simulated controller, its behaviour a rules file, riding a channel.
//!
//! Nothing here names a device, a vendor or a product: what a controller's telegrams mean is
//! configuration (a channel file, a catalog, a rules file), as in core (`docs/questions.md` Q8).

pub mod channel;
pub mod controller;
pub mod emit;
pub mod opc;
