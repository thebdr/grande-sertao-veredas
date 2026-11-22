//! What points 4, 5 and 7 stand on: Qt WebEngine draws a page on the `offscreen` platform, and a
//! test reads the page through a hook on Oficina's side (`runJavaScript` from QML), never through a
//! script the page carries.

mod common;

use common::{Shell, X};

#[test]
fn an_embedded_page_loads_with_no_display_and_is_read_back_through_oficina_s_hook() {
    let shell = Shell::start(&[], &[]);
    shell.wait_for("oficina/web", "loadProgress", "100");
    shell.wait_for("oficina/web", "title", "probe");
    shell.invoke("oficina/webPane", "readText", &[X::Str("x".into())]);
    shell.wait_for("oficina/webPane", "pageText", "hello from the page");
    // An element the page does not hold reads as nothing.
    shell.invoke("oficina/webPane", "readText", &[X::Str("absent".into())]);
    shell.wait_for("oficina/webPane", "pageText", "");
}
