//! The table behind point 3's view: a million rows held as a size and a seed, every cell computed
//! from its row alone.

use oficina_spike_model::table::{COLUMNS, Synthetic};

#[test]
fn a_cell_is_computed_from_its_row_alone_and_reads_the_same_whatever_was_read_before() {
    let t = Synthetic::new(1_000_000, 2026);
    assert_eq!((t.rows(), t.columns()), (1_000_000, 8));
    assert_eq!(COLUMNS.iter().enumerate().map(|(c, _)| t.header(c)).collect::<Vec<_>>(), COLUMNS.map(Some).to_vec());
    assert_eq!(t.header(8), None);
    // The row and the time are the row's own: 100 ms a row, hours past 24.
    assert_eq!(t.cell(0, 0).as_deref(), Some("0"));
    assert_eq!(t.cell(0, 1).as_deref(), Some("00:00:00.000"));
    assert_eq!(t.cell(1, 1).as_deref(), Some("00:00:00.100"));
    assert_eq!(t.cell(36_000, 1).as_deref(), Some("01:00:00.000"));
    assert_eq!(t.cell(999_999, 0).as_deref(), Some("999999"));
    assert_eq!(t.cell(999_999, 1).as_deref(), Some("27:46:39.900"));
    // A row read forwards, backwards, or one cell alone, reads the same.
    let forwards: Vec<String> = (0..8).map(|c| t.cell(123_456, c).expect("a cell")).collect();
    let backwards: Vec<String> = (0..8).rev().map(|c| t.cell(123_456, c).expect("a cell")).rev().collect();
    assert_eq!(forwards, backwards);
    assert_eq!(t.cell(123_456, 6), Some(forwards[6].clone()));
    // Each value within its column's range and written as its column writes it.
    for row in [0, 1, 2, 999_998, 999_999] {
        let cells: Vec<String> = (0..8).map(|c| t.cell(row, c).expect("a cell")).collect();
        let line: i64 = cells[2].parse().expect("an integer");
        let station: i64 = cells[3].parse().expect("an integer");
        let weight: f64 = cells[4].parse().expect("a number");
        let speed: f64 = cells[5].parse().expect("a number");
        let count: i64 = cells[6].parse().expect("an integer");
        assert!((1..=12).contains(&line) && (1..=240).contains(&station) && (0..=99_999).contains(&count), "{cells:?}");
        assert!((0.2..=35.0).contains(&weight) && cells[4].split('.').nth(1).map(str::len) == Some(2), "{cells:?}");
        assert!((0.5..=2.5).contains(&speed) && cells[5].split('.').nth(1).map(str::len) == Some(3), "{cells:?}");
        assert!(cells[7] == "true" || cells[7] == "false", "{cells:?}");
    }
    // Rows differ, and another seed is another table.
    assert_ne!((2..8).map(|c| t.cell(0, c)).collect::<Vec<_>>(), (2..8).map(|c| t.cell(1, c)).collect::<Vec<_>>());
    assert_ne!(Synthetic::new(10, 1).cell(0, 6), Synthetic::new(10, 2).cell(0, 6));
    // Nothing outside the table.
    assert_eq!((t.cell(1_000_000, 0), t.cell(0, 8)), (None, None));
    assert_eq!(Synthetic::new(0, 1).cell(0, 0), None);
}
