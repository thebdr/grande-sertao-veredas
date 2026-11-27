//! A program whose own crate declares a trait named `Program` and implements it, beside its one `orcl_std::Program`:
//! the front end takes the trait of that name in orcl-std's crate alone.
#![forbid(unsafe_code)]

use orcl_std::Scan;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub v: u8,
}

pub mod recipe {
    /// A recipe's own program of steps: the plant's word, not the controller's.
    pub trait Program {
        fn step(&self, at: u8) -> u8;
    }

    #[derive(Debug, Default)]
    pub struct Mix;

    impl Program for Mix {
        fn step(&self, at: u8) -> u8 {
            at.wrapping_mul(3)
        }
    }
}

use recipe::Program as _;

#[derive(Debug, Default)]
pub struct OwnTrait {
    mix: recipe::Mix,
}

impl orcl_std::Program for OwnTrait {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.v = self.mix.step(i.a);
    }
}
