//! # orcl-runtime - the engine of an ORCL scan
//!
//! Runs one program as a cyclic [`Task`] on a [`Clock`] its host hands in, so the same scan runs on a target's
//! monotonic clock ([`MonoClock`]) and on virtual time in a test or a simulation ([`VirtualClock`]). The program
//! is a [`TaskProgram`]: a native [`orcl_std::Program`], which never fails, or an [`Interpreted`] one, a machine of
//! `orcl-interp` whose scan may end in a program error:
//! - **absolute deadlines.** Scan `k` is due at `k * period` from the start and starts there, whatever the scans
//!   before it took. A deadline that passed while a scan ran is skipped and counted, never caught up;
//! - **a process image.** The inputs are read once at the start of a scan, the program runs on that snapshot and
//!   the scan's time, and the outputs are written once at its end;
//! - **the time-error rule** of `docs/controlador-plan.md` ("Faults"): a scan longer than its period is a time
//!   error, reported with the scan for the host's time-error handler; a scan longer than twice its period faults
//!   the task, its own outputs never written, and from then on every scan writes the program's safe outputs
//!   without scanning it, so the plant's exchange goes on; a scan whose program ends in an error faults it so too;
//! - **cycle statistics:** scans, overruns, skipped cycles, each scan's lateness against its deadline and its
//!   length.
//!
//! One cyclic task only: several tasks, pre-empting one another on one logic thread as controlador's execution
//! model rules (`docs/questions.md` Q223 with Q33), are not built yet. No operating-system code: the host that
//! runs a program on a target sets its own process up for real time (`docs/orcl/README.md`). The crate is
//! `no_std` with std brought in by name, so every use of std is written out (`std::time::Instant`,
//! `std::thread::sleep`, `std::rc::Rc`) and no macro of std's prelude, a print to the terminal among them, comes
//! in unseen.
#![no_std]
#![forbid(unsafe_code)]

extern crate std;

mod clock;
pub mod interpreted;
mod task;

pub use clock::{Clock, MonoClock, VirtualClock};
pub use interpreted::{Image, Interpreted, MakeError, Regions, ScanError};
pub use task::{CycleStats, Fault, Io, Report, RunState, Task, TaskConfig, TaskError, TaskProgram};
