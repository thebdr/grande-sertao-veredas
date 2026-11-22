//! The scan model: what a program sees each cycle, and what a program is.
use core::time::Duration;

/// One scan of a task. `now` is the time since the runtime started, read ONCE at the start of the scan; every
/// time-dependent block takes its time from here, never from a clock. `cycle` is the scan's sequence number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    pub now: Duration,
    pub period: Duration,
    pub cycle: u64,
}

/// A control program, run once per scan.
///
/// It is given a consistent snapshot of its inputs (read-only for the whole scan) and writes its outputs; the
/// runtime applies the outputs after the scan. The program owns all of its state.
pub trait Program {
    type Inputs: Copy + Default;
    type Outputs: Copy + Default;

    fn scan(&mut self, scan: &Scan, inputs: &Self::Inputs, outputs: &mut Self::Outputs);

    /// The outputs the runtime applies while the task is faulted: the safe state. All-default unless the
    /// program says otherwise.
    fn safe_outputs(&self) -> Self::Outputs {
        Self::Outputs::default()
    }
}

/// `ms(500)`: a duration in milliseconds.
pub const fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

/// `s(2)`: a duration in seconds.
pub const fn s(value: u64) -> Duration {
    Duration::from_secs(value)
}
