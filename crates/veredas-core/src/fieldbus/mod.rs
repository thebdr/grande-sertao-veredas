//! The fieldbus masters: the functions that drive a plant's devices over a standard fieldbus.
//!
//! A standard fieldbus's frames are fixed by its standard, so they are code here, unlike a
//! telegram channel's layout, which is configuration (`docs/questions.md` Q9, revisited). No
//! vendor's device or catalog is named. These functions drive hardware: they are meant for a
//! controller on its real-time target, and other crates call them for testing only (a rule in
//! prose, `docs/core-plan.md`).
//!
//! Each master is a feature of the crate, off by default, so a component that never drives
//! hardware carries none of their code or dependencies:
//!
//! - `modbus` ([`modbus`]): a Modbus/TCP client, every public function of a device's data model,
//!   each answer checked against its request and every call bounded in time.

#[cfg(feature = "modbus")]
pub mod modbus;
