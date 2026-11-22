//! The spike's point 3: a million rows from a Rust model, the view asking only for what it draws.

mod common;

use std::time::{Duration, Instant};

use common::{Shell, X};
use oficina_spike_model::table::Synthetic;

const SEED: u64 = 2026;

/// Open a table of `rows` rows and bring its last row into view; how long each took.
fn open_and_reach_the_end(shell: &Shell, rows: usize) -> (Duration, Duration) {
    shell.wait_for("oficina/openTable", "visible", "true");
    let start = Instant::now();
    shell.click("oficina/openTable");
    shell.wait_for("oficina/cell_0_0", "display", "0");
    let opened = start.elapsed();
    shell.wait_for("oficina/tableRows", "text", &rows.to_string());
    let last = rows - 1;
    let start = Instant::now();
    shell.invoke("oficina/tablePane", "jumpTo", &[X::Int(last as i64), X::Int(0)]);
    shell.wait_for(&format!("oficina/cell_{last}_0"), "display", &last.to_string());
    (opened, start.elapsed())
}

#[test]
fn a_million_rows_open_and_the_last_shows_the_model_s_own_cells() {
    let shell = Shell::start(&["--table-rows=1000000"], &[]);
    let (opened, jumped) = open_and_reach_the_end(&shell, 1_000_000);
    eprintln!("a million rows: opened in {opened:?}, the last row drawn in {jumped:?}");
    // Every column of the last row, each brought into view, shows the model's own cell.
    let model = Synthetic::new(1_000_000, SEED);
    for column in 0..8 {
        shell.invoke("oficina/tablePane", "jumpTo", &[X::Int(999_999), X::Int(column as i64)]);
        let expected = model.cell(999_999, column).expect("a cell");
        shell.wait_for(&format!("oficina/cell_999999_{column}"), "display", &expected);
    }
    assert_eq!(shell.property("oficina/cell_999999_1", "display"), "27:46:39.900");
    // Only what is drawn exists: the first row left the view with its cells.
    assert!(!shell.visible("oficina/cell_0_0"), "row 0 is no longer drawn");
}

/// A timing of the product, judged on a release build alone on the machine.
#[test]
#[cfg_attr(debug_assertions, ignore = "a timing: run it with --release")]
fn a_million_rows_open_in_under_a_second() {
    let shell = Shell::start(&["--table-rows=1000000"], &[]);
    let (opened, jumped) = open_and_reach_the_end(&shell, 1_000_000);
    eprintln!("a million rows (release): opened in {opened:?}, the last row drawn in {jumped:?}");
    assert!(opened < Duration::from_secs(1), "opened in {opened:?}");
    assert!(jumped < Duration::from_secs(1), "the last row in {jumped:?}");
}

/// What the view holds does not grow with the table: a million rows cost what a thousand do,
/// within 20 MB, once each table is open and its last row drawn.
#[cfg(target_os = "linux")]
#[test]
fn a_million_rows_cost_the_memory_a_thousand_do() {
    let resident = |rows: usize| {
        let shell = Shell::start(&[&format!("--table-rows={rows}")], &[]);
        open_and_reach_the_end(&shell, rows);
        std::thread::sleep(Duration::from_millis(500));
        shell.resident_kib()
    };
    let thousand = resident(1_000);
    let million = resident(1_000_000);
    eprintln!("resident memory: a thousand rows {thousand} KiB, a million {million} KiB");
    assert!(million < thousand + 20 * 1024, "a thousand rows {thousand} KiB, a million {million} KiB");
}
