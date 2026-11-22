//! The spike's point 6, as far as one machine with a GPU goes: Oficina finds what draws it before
//! its first window, says so in its footer, keeps its effects to a GPU, and starts WebEngine without
//! the GPU when a processor draws; and point 3's frame times, scrolling a million rows.
//!
//! A run names what it expects: `OFICINA_EXPECT_DRAWING=software` (Mesa's rasteriser forced with
//! `LIBGL_ALWAYS_SOFTWARE=1`, or a machine with no GPU) or `gpu`, and `OFICINA_EXPECT_ADAPTER` a
//! word of the adapter's name (`llvmpipe`, say). Unnamed, the test holds only what holds anywhere:
//! effects on exactly where a GPU draws.

mod common;

use std::time::Duration;

use common::{Shell, X};

/// What draws a page's WebGL, read in the page through Oficina's hook.
const WEBGL: &str = "(function () { const gl = document.createElement('canvas').getContext('webgl'); if (!gl) return 'no WebGL'; const i = gl.getExtension('WEBGL_debug_renderer_info'); return i ? gl.getParameter(i.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER); })()";

/// Names that are a GPU's, in a renderer string.
const GPUS: [&str; 5] = ["Intel", "NVIDIA", "AMD", "Radeon", "Apple"];

#[test]
fn the_shell_names_what_draws_it_and_keeps_its_effects_to_a_gpu() {
    let shell = Shell::start(&[], &[]);
    let adapter = shell.property("oficina/drawing", "adapter");
    let api = shell.wait_until("oficina/drawing", "api", Duration::from_secs(10), |a| a != "unknown", "Qt Quick's API");
    let software = shell.property("oficina/drawing", "software") == "true";
    let shadowed = shell.property("oficina/drawing", "shadowed") == "true";
    let footer = shell.property("oficina/drawing", "text");
    shell.wait_for("oficina/web", "loadProgress", "100");
    let webgl = shell.eval("oficina/webPane", WEBGL);
    eprintln!("adapter {adapter:?}, software {software}, Qt Quick through {api}, effects {shadowed}; a page's WebGL: {webgl}; the footer: {footer:?}");
    assert_eq!(shadowed, !software, "effects on exactly where a GPU draws");
    assert!(footer.starts_with(if software { "Drawing in software, " } else { "Drawing on " }), "{footer}");
    match std::env::var("OFICINA_EXPECT_DRAWING").as_deref() {
        Ok("software") => {
            assert!(software, "software drawing not found: {adapter}");
            assert!(footer.ends_with(": effects off, pages drawn without the GPU"), "{footer}");
            assert!(!GPUS.iter().any(|g| webgl.contains(g)), "a page drew through a GPU: {webgl}");
        }
        Ok("gpu") => assert!(!software, "a GPU not found: {adapter}"),
        Ok(other) => panic!("OFICINA_EXPECT_DRAWING is `software` or `gpu`, not `{other}`"),
        Err(_) => {}
    }
    if let Ok(word) = std::env::var("OFICINA_EXPECT_ADAPTER") {
        assert!(adapter.contains(&word), "the adapter {adapter:?} does not name {word:?}");
    }
    assert_eq!(shell.errors(), Vec::<String>::new());
}

/// Point 3's frame times: a million rows scrolled steadily, 500 rows a second for 3 s, each frame's
/// interval taken; recorded, not judged (the plan's point 6).
#[test]
#[cfg_attr(debug_assertions, ignore = "a timing: run it with --release")]
fn a_million_rows_scrolled_frame_by_frame() {
    let shell = Shell::start(&[], &[]);
    shell.click("oficina/openTable");
    shell.wait_for("oficina/tableRows", "text", "1000000");
    shell.invoke("oficina/tablePane", "scrollFor", &[X::Int(3000), X::Int(500)]);
    let frames = shell.wait_until("oficina/tablePane", "frames", Duration::from_secs(20), |f| !f.is_empty(), "the frame times");
    eprintln!("scrolled 500 rows a second for 3 s; frame intervals in ms {frames}; {}", shell.property("oficina/drawing", "text"));
    let f: serde_json::Value = serde_json::from_str(&frames).expect("JSON");
    assert!(f["frames"].as_u64().is_some_and(|n| n > 0), "no frame drawn while scrolling: {frames}");
}
