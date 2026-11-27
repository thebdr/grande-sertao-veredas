//! A conformance program: ordinary control code the front end does not refuse, run lowered beside native.
#![forbid(unsafe_code)]

use core::time::Duration;
use orcl_std::{Program, Scan};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inputs {
    pub a: i8,
    pub b: u64,
    pub sel: u8,
    pub on: bool,
    pub off: bool,
    pub t_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outputs {
    pub tuple_match: u8,
    pub found: u8,
    pub broke: u32,
    pub nested: u16,
    pub items: u32,
    pub xor: bool,
    pub bool_ne: bool,
    pub widened: u64,
    pub narrowed: i8,
    pub cleared: bool,
    pub early: u8,
    pub defaulted: u32,
    pub cloned: u16,
    pub guard: u8,
    pub labeled: u8,
    pub since_ms: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Idle,
    Run { speed: u8, dir: bool },
    Stop(u16),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Item {
    pub count: u32,
    pub weight: u16,
}

#[derive(Debug)]
pub struct Sound {
    items: [Item; 3],
    mode: Option<Mode>,
    since: Option<Duration>,
    total: u32,
}

impl Default for Sound {
    fn default() -> Sound {
        Sound {
            items: [Item { count: 0, weight: 2 }, Item { count: 1, weight: 3 }, Item { count: 2, weight: 5 }],
            mode: None,
            since: None,
            total: Default::default(),
        }
    }
}

const fn doubled(v: u8) -> u8 {
    v.wrapping_mul(2)
}

fn find(values: &[u8; 5], wanted: u8) -> u8 {
    let mut k = 0u8;
    while k < 5 {
        if values[k as usize] == wanted {
            return k;
        }
        k += 1;
    }
    255
}

impl Item {
    fn weighed(self) -> u32 {
        self.count * self.weight as u32
    }
}

impl Program for Sound {
    type Inputs = Inputs;
    type Outputs = Outputs;

    fn scan(&mut self, scan: &Scan, i: &Inputs, o: &mut Outputs) {
        o.tuple_match = match (i.on, i.off) {
            (true, false) => 1,
            (false, true) => 2,
            (true, true) => 3,
            (false, false) => 4,
        };
        o.found = find(&[3, 1, 4, 1, 5], i.sel % 7);
        let mut n = 0u32;
        let broke = loop {
            n += 1;
            if n * n > i.sel as u32 {
                break n;
            }
        };
        o.broke = broke;
        self.mode = match i.sel % 4 {
            0 => None,
            1 => Some(Mode::Idle),
            2 => Some(Mode::Run { speed: i.sel, dir: i.on }),
            _ => Some(Mode::Stop(i.sel as u16 * 9)),
        };
        o.nested = match self.mode {
            Some(Mode::Run { speed, dir: true }) => speed as u16,
            Some(Mode::Run { speed, .. }) => 300 + speed as u16,
            Some(Mode::Stop(code)) if code > 500 => 1,
            Some(Mode::Stop(code)) => code,
            Some(Mode::Idle) => 2,
            None => 3,
        };
        self.items[(i.sel % 3) as usize].count += 1;
        self.items[(i.sel % 3) as usize].weight = self.items[(i.sel % 3) as usize].weight.wrapping_add(i.a as u16);
        o.items = self.items[0].weighed().wrapping_add(self.items[1].weighed()).wrapping_add(self.items[2].weighed());
        o.xor = i.on ^ i.off;
        o.bool_ne = i.on != i.off;
        o.widened = i.a as u64;
        o.narrowed = i.b as i8;
        o.cleared = !(i.on || i.off) && i.sel > 100;
        o.early = doubled(i.sel);
        self.total = self.total.wrapping_add(1);
        o.defaulted = self.total;
        let item = self.items[1];
        // the derived `Clone` called on purpose, lowered as the program's own function
        #[allow(clippy::clone_on_copy)]
        let cloned = item.clone();
        o.cloned = cloned.weight;
        o.guard = match i.sel {
            0..=9 => 1,
            10..=99 if i.on => 2,
            10..=99 => 3,
            _ => 4,
        };
        let mut found = 0u8;
        for_each_row(&mut found, i.sel);
        o.labeled = found;
        if i.sel == 42 {
            return;
        }
        let now = scan.now;
        let since = *self.since.get_or_insert(now);
        o.since_ms = now.saturating_sub(since) > Duration::from_millis(i.t_ms);
    }
}

fn for_each_row(found: &mut u8, sel: u8) {
    let mut r = 0u8;
    'rows: while r < 4 {
        let mut c = 0u8;
        while c < 4 {
            if r * 4 + c == sel % 16 {
                *found = r * 10 + c;
                break 'rows;
            }
            c += 1;
        }
        r += 1;
    }
}
