//! The driver itself (the spike's point 7, its first step): a Rust test starts the shell with no
//! display and drives a QML control inside a docked pane by its name.

mod common;

use common::Shell;

#[test]
fn a_test_clicks_a_control_inside_a_docked_pane_and_reads_what_it_did() {
    let shell = Shell::start(&[], &[]);
    assert!(shell.visible("oficina/hello"), "the button is drawn");
    shell.wait_for("oficina/hello", "text", "clicked 0");
    for _ in 0..3 {
        shell.click("oficina/hello");
    }
    shell.wait_for("oficina/hello", "text", "clicked 3");
    assert_eq!(shell.errors(), Vec::<String>::new());
}
