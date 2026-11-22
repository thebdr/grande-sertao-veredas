//! # orcl-runtime - the engine of an ORCL scan
//!
//! Runs one [`orcl_std::Program`] as a cyclic task:
//! - **absolute deadlines.** Scan `k` is due at `k * period` from the start, so the cycle never drifts. When
//!   deadlines are missed, the skipped cycles are counted and the task resumes on the next due one;
//! - **a process image.** The task reads its inputs once at the start of each scan, the program runs on that
//!   snapshot, and the outputs are applied after the scan;
//! - **a watchdog.** After `overrun_limit` consecutive cycles that ran longer than the period, the task faults:
//!   from then on it applies the program's safe outputs;
//! - **cycle statistics:** jitter (actual start vs deadline), execution time, overruns, skipped cycles.
//!
//! Carried from Veredas 1 (`docs/orcl/README.md`, "What it absorbs from Veredas 1"). Its real-time setup of a
//! Linux process is not carried: the language's crates hold no operating-system code, and the host that runs a
//! program on a target sets its own process up. The task runs on the operating system's sleep, then a short spin
//! to each deadline.
#![forbid(unsafe_code)]

mod clock;
mod task;

pub use clock::MonoClock;
pub use task::{CycleStats, Io, Report, RunState, TaskConfig, run};
