//! A table of a million rows computed when asked (the spike's point 3): nothing is held but its
//! size and a seed, and a cell is computed from its row alone, each row drawing from its own
//! stream of core's PCG32, so a cell reads the same whatever was read before it and a view that
//! asks only for the visible cells costs only them. The store explorer's result grid will sit
//! behind the same seam, its cells from a query's window instead.

use veredas_core::rng::Pcg32;

/// The columns, in order.
pub const COLUMNS: [&str; 8] = ["row", "time", "line", "station", "weight", "speed", "count", "ok"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Synthetic {
    rows: usize,
    seed: u64,
}

impl Synthetic {
    pub fn new(rows: usize, seed: u64) -> Synthetic {
        Synthetic { rows, seed }
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn columns(&self) -> usize {
        COLUMNS.len()
    }

    pub fn header(&self, column: usize) -> Option<&'static str> {
        COLUMNS.get(column).copied()
    }

    /// The cell's text, or nothing outside the table. The time is the row's offset from the start
    /// of the run, 100 ms a row, as hours (past 24), minutes, seconds and milliseconds.
    pub fn cell(&self, row: usize, column: usize) -> Option<String> {
        if row >= self.rows || column >= COLUMNS.len() {
            return None;
        }
        let ms = row as u64 * 100;
        if column < 2 {
            return Some(match column {
                0 => row.to_string(),
                _ => format!("{:02}:{:02}:{:02}.{:03}", ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000),
            });
        }
        // The draws come in a fixed order, so column k is the same whichever columns are asked.
        let mut g = Pcg32::new(self.seed, row as u64);
        let line = g.int_in(1, 12);
        let station = g.int_in(1, 240);
        let weight = g.uniform(0.2, 35.0, 2);
        let speed = g.uniform(0.5, 2.5, 3);
        let count = g.int_in(0, 99_999);
        let ok = g.flip();
        Some(match column {
            2 => line.to_string(),
            3 => station.to_string(),
            4 => format!("{weight:.2}"),
            5 => format!("{speed:.3}"),
            6 => count.to_string(),
            _ => ok.to_string(),
        })
    }
}
