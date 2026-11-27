//! A program's own `Option`, in a module, in a program that holds no `Option` of core's.
#![forbid(unsafe_code)]
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: u8,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub v: u8,
}

pub mod mine {
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum Option {
        #[default]
        None,
        Some(u8),
    }
}

#[derive(Debug, Default)]
pub struct QNoopt {
    last: mine::Option,
}

impl Program for QNoopt {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, _scan: &Scan, i: &Inputs, o: &mut Outputs) {
        let before = self.last;
        self.last = if i.a % 2 == 0 { mine::Option::None } else { mine::Option::Some(i.a) };
        o.v = match before {
            mine::Option::None => 0,
            mine::Option::Some(v) => v,
        };
    }
}
