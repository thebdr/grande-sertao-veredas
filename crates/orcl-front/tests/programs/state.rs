//! The program's state named apart, a `Plant` in two modules, the state the first: its entries named by the state's
//! final name, `<line::Plant as Program>::scan`.
#![forbid(unsafe_code)]
pub use line::Plant;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub v: u8,
    pub w: u8,
}

pub mod other {
    #[derive(Debug, Default)]
    pub struct Plant {
        pub n: u8,
    }
}

pub mod line {
    use super::{Inputs, Outputs};
    use orcl_std::{Program, Scan};

    #[derive(Debug, Default)]
    pub struct Plant {
        pub k: u8,
        pub inner: super::other::Plant,
    }

    impl Program for Plant {
        type Inputs = Inputs;
        type Outputs = Outputs;

        fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
            self.k = self.k.wrapping_add(i.a);
            self.inner.n = self.inner.n.wrapping_add(1);
            o.v = self.k;
            o.w = self.inner.n;
        }
    }
}
