//! The scan model: what a program sees each cycle, and what a program is.
//!
//! A program's inputs and outputs are values, copied into and out of the process image: both must be `Copy`. A
//! program whose inputs or outputs are not does not compile, even with every other trait (a `Vec` below: `Clone`,
//! `Default`, `Debug`, `PartialEq`...). Each `compile_fail` has a twin that compiles, since stable rustdoc does not
//! check the error code it names:
//!
//! ```
//! use orcl_std::{Program, Scan};
//! struct P;
//! impl Program for P {
//!     type Inputs = [u8; 4];
//!     type Outputs = [u8; 4];
//!     fn scan(&mut self, _: &Scan, i: &[u8; 4], o: &mut [u8; 4]) { *o = *i; }
//! }
//! ```
//!
//! A program on a float, and on a structure that is `Copy` and `Default` and nothing else, compiles too: the bounds
//! ask for no more.
//!
//! ```
//! use orcl_std::{Program, Scan};
//! #[derive(Clone, Copy, Default)]
//! struct Valve { open: bool, position: f32 }
//! struct P;
//! impl Program for P {
//!     type Inputs = f32;
//!     type Outputs = Valve;
//!     fn scan(&mut self, _: &Scan, level: &f32, valve: &mut Valve) { valve.open = *level > 0.5; valve.position = *level; }
//! }
//! ```
//!
//! Nor is a value asked anything else: a program on a value that is `Copy` and `Default` and nothing more (not
//! `Send`, `Sync`, `'static`, `Unpin`, `UnwindSafe` or `RefUnwindSafe`) compiles, and is scanned and asked its safe
//! outputs, through a generic runner and directly.
//!
//! ```
//! use core::marker::{PhantomData, PhantomPinned};
//! use orcl_std::{Program, Scan};
//! #[derive(Clone, Copy, Default)]
//! struct Bare<'a>(PhantomData<(&'a mut (), core::cell::Cell<()>, *const ())>, PhantomPinned);
//! struct P<'a>(PhantomData<&'a ()>);
//! impl<'a> Program for P<'a> {
//!     type Inputs = Bare<'a>;
//!     type Outputs = Bare<'a>;
//!     fn scan(&mut self, _: &Scan, i: &Bare<'a>, o: &mut Bare<'a>) { *o = *i; }
//! }
//! fn run<Q: Program>(program: &mut Q) -> Q::Outputs {
//!     let mut outputs = Q::Outputs::default();
//!     program.scan(&Scan::default(), &Q::Inputs::default(), &mut outputs);
//!     program.safe_outputs()
//! }
//! let mut program = P(PhantomData);
//! let _ = run(&mut program);
//! let mut outputs = Bare::default();
//! program.scan(&Scan::default(), &Bare::default(), &mut outputs);
//! let _ = program.safe_outputs();
//! ```
//!
//! ```compile_fail,E0277
//! use orcl_std::{Program, Scan};
//! struct P;
//! impl Program for P {
//!     type Inputs = Vec<u8>;
//!     type Outputs = [u8; 4];
//!     fn scan(&mut self, _: &Scan, _: &Vec<u8>, _: &mut [u8; 4]) {}
//! }
//! ```
//!
//! ```compile_fail,E0277
//! use orcl_std::{Program, Scan};
//! struct P;
//! impl Program for P {
//!     type Inputs = [u8; 4];
//!     type Outputs = Vec<u8>;
//!     fn scan(&mut self, _: &Scan, _: &[u8; 4], _: &mut Vec<u8>) {}
//! }
//! ```
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
