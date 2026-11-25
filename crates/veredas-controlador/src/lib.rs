//! # veredas-controlador - the soft PLC
//!
//! Runs an ORCL program in a deterministic scan and keeps the controller's state as structured data, core's
//! [`veredas_core::state::ControllerState`], whose text has one writer and one reader in core
//! (`docs/controlador-plan.md`, "The state"). This first build holds the [`Controller`]: its states (STOP,
//! STARTUP, RUN, FAULT), the commands that move it (start, stop, acknowledge) and the gate on them (the
//! commissioning switch, a person and not the program), its start after a power cycle, and one cyclic task on
//! `orcl-runtime`'s engine running a native program or an interpreted one ([`Loaded`]). HOLD, forcing, debugging,
//! handlers, retained data, the program file's load, the fieldbus and the embedded portal come with their own
//! clauses.
#![forbid(unsafe_code)]

mod controller;

pub use controller::{Command, Controller, Loaded, Native, PowerOnError, Refusal, Restart, Source, Switch, Target};
