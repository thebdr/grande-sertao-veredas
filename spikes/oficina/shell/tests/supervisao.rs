//! The spike's point 5: a supervisão page embedded and read back. A real supervisão host, started
//! by the test on the plant supervisão's own tests meet (the reference's worked example,
//! `docs/binding-schema.md`, its project pointed at the bench's OPC UA server); the page it serves
//! shown in Oficina's web pane; the loop's speed, driven by the bench and read by portal, read from
//! the page through Oficina's hook; and the served page finding no bridge object, where the page
//! Oficina ships finds one.
//!
//! The host is supervisão's own executable, built in the repository's target folder
//! (`cargo build -p veredas-supervisao --bin veredas-supervisao` at the repository's root);
//! `OFICINA_SUPERVISAO` names another. A missing one is a failure, never a skip.

mod common;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use common::{Shell, X};
use veredas_maquinar::bench::{self, Bench};
use veredas_maquinar::opc::render_value;

fn doc(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs").join(name);
    std::fs::read_to_string(p).unwrap().replace("\r\n", "\n")
}

/// The fenced blocks of a reference in a language, in order.
fn blocks(name: &str, lang: &str) -> Vec<String> {
    let text = doc(name);
    let fence = format!("```{lang}\n");
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find(&fence) {
        let body = &rest[start + fence.len()..];
        let end = body.find("```").unwrap();
        out.push(body[..end].to_string());
        rest = &body[end + 3..];
    }
    out
}

/// The bench of the worked example, as supervisão's tests start it: an OPC UA server `plc` holding
/// the example's tags on a port the system picks, every value set by the test.
fn plant(dir: &Path) -> Bench {
    let text = "format = \"veredas-bench\"\nversion = 1\nseed = 5\n\n[[opc]]\nname = \"plc\"\nport = 0\ntags = [\n  { id = \"speed\", type = \"float\", value = \"1.5\" },\n  { id = \"p1\", type = \"float\", value = \"25\" },\n  { id = \"p2\", type = \"float\", value = \"75\" },\n  { id = \"fill1\", type = \"float\", value = \"35\" },\n  { id = \"fill2\", type = \"float\", value = \"85\" },\n  { id = \"fill3\", type = \"float\", value = \"100\" },\n  { id = \"infeed\", type = \"int\", value = \"1\" },\n  { id = \"release3\", type = \"int\", value = \"0\", access = \"read-write\" },\n]\n";
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("bench.toml"), text).unwrap();
    Bench::start(bench::load(text, dir).unwrap()).unwrap()
}

/// The worked example written into `dir`, pointed at the running bench: its endpoint, the namespace
/// the server assigned, the host on a port the system picks, the poll at 50 ms, writes not armed.
fn write_example(dir: &Path, bench: &Bench) -> PathBuf {
    let plc = bench.server("plc").unwrap();
    let t = blocks("binding-schema.md", "toml");
    let s = blocks("binding-schema.md", "svg");
    assert_eq!((t.len(), s.len()), (3, 1), "the reference holds three TOML blocks and one SVG block");
    let host = t[0].replace("bind = \"127.0.0.1:8902\"", "bind = \"127.0.0.1:0\"");
    let project = t[1]
        .replace("opc.tcp://127.0.0.1:4855/", &plc.endpoint_url)
        .replace("ns=2;", &format!("ns={};", plc.namespace))
        .replace("poll_ms = 200", "poll_ms = 50");
    let binding = t[2].replace("ns=2;", &format!("ns={};", plc.namespace));
    std::fs::create_dir_all(dir.join("loop")).unwrap();
    std::fs::write(dir.join("supervisao.toml"), host).unwrap();
    std::fs::write(dir.join("loop/project.toml"), project).unwrap();
    std::fs::write(dir.join("loop/binding.toml"), binding).unwrap();
    std::fs::write(dir.join("loop/layout.svg"), &s[0]).unwrap();
    dir.join("supervisao.toml")
}

/// Supervisão's host, running until its stdin closes.
struct Host {
    child: Child,
    stdin: Option<ChildStdin>,
    addr: String,
}

impl Host {
    fn start(file: &Path) -> Host {
        let exe = std::env::var_os("OFICINA_SUPERVISAO")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/veredas-supervisao"));
        assert!(exe.is_file(), "no supervisão host at {}: build it with `cargo build -p veredas-supervisao --bin veredas-supervisao` at the repository's root, or name one in OFICINA_SUPERVISAO", exe.display());
        let mut child = Command::new(&exe).arg(file).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().unwrap();
        let stdin = child.stdin.take();
        let mut out = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        let addr = loop {
            line.clear();
            assert!(out.read_line(&mut line).unwrap() > 0, "the host ended before serving");
            if let Some(rest) = line.trim().strip_prefix("serving on http://") {
                break rest.trim_end_matches('/').to_string();
            }
        };
        // What it prints afterwards is drained, so a full pipe never holds it.
        std::thread::spawn(move || std::io::copy(&mut out, &mut std::io::sink()));
        Host { child, stdin, addr }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stdin.take();
        let until = Instant::now() + Duration::from_secs(10);
        while Instant::now() < until {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The headline's figure for the loop's speed, read through Oficina's hook (its own world): its text
/// and its class, or null while the page does not show it.
const SPEED: &str = "(function () { const f = document.querySelector('.headline .fig[data-object=\"speed\"] .mono'); return f ? [f.textContent, f.className] : null; })()";

/// Read `expression` from the web pane until `ok` holds, within `within`; the last reading.
fn read_until(shell: &Shell, expression: &str, within: Duration, ok: impl Fn(&str) -> bool) -> String {
    let until = Instant::now() + within;
    loop {
        let got = shell.eval("oficina/webPane", expression);
        if ok(&got) {
            return got;
        }
        assert!(Instant::now() < until, "the page never showed it: last read {got}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn a_supervisao_page_embedded_shows_what_the_bench_drove_and_finds_no_bridge() {
    let dir = tempfile::Builder::new().prefix("supervisao-").tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let bench = plant(&dir.path().join("bench"));
    let plc = bench.server("plc").unwrap();
    let host = Host::start(&write_example(&dir.path().join("site"), &bench));
    let page = format!("http://{}/s/loop/", host.addr);

    let shell = Shell::start(&[], &[]);
    // The control: the page Oficina ships, joined by its web channel, finds the bridge object.
    shell.wait_for("oficina/codeView", "loadProgress", "100");
    let bridge = "typeof qt === 'undefined' ? 'none' : (qt.webChannelTransport ? 'bridge' : 'qt, no bridge')";
    assert_eq!(shell.eval("oficina/codePane", bridge), "\"bridge\"", "the page Oficina ships finds its bridge");

    let asked = Instant::now();
    shell.invoke("oficina/webPane", "load", &[X::Str(page.clone())]);
    shell.wait_until("oficina/web", "url", Duration::from_secs(10), |u| u.starts_with(&format!("http://{}/", host.addr)), "the served page's address");
    shell.wait_for("oficina/web", "loading", "false");
    // What the bench holds, as the binding writes it (one decimal), and what the page shows.
    let shown = |v: f64| format!("{v:.1}");
    let driven = || render_value(&plc.get("speed")).parse::<f64>().unwrap();
    let first = read_until(&shell, SPEED, Duration::from_secs(20), |g| g != "null");
    eprintln!("the headline first read {first}, {} ms after the page was asked for", asked.elapsed().as_millis());
    read_until(&shell, SPEED, Duration::from_secs(20), |g| g.starts_with(&format!("[\"{}\",", shown(driven()))));

    // The bench drives the speed; portal reads it; the page follows, read back each time.
    for v in [0.2, 2.7, 1.5] {
        plc.set("speed", v);
        let set = Instant::now();
        let want = shown(driven());
        assert_eq!(want, shown(v), "the bench holds what it was set to");
        let got = read_until(&shell, SPEED, Duration::from_secs(20), |g| g.starts_with(&format!("[\"{want}\",")));
        eprintln!("speed {v}: the page read {got} {} ms after the bench set it", set.elapsed().as_millis());
    }

    // The served page, in its own world, finds no bridge object: none is offered to a page Oficina
    // does not ship (the review's entry 32).
    assert_eq!(shell.eval_with("oficina/webPane", "evaluateInPage", bridge), "\"none\"", "the served page finds no bridge");
    // And Oficina's hook ran in a world of its own: what it defines the page does not see.
    shell.eval("oficina/webPane", "(window.oficinaHook = 1)");
    assert_eq!(shell.eval_with("oficina/webPane", "evaluateInPage", "typeof window.oficinaHook"), "\"undefined\"", "the hook's world is not the page's");
    assert_eq!(shell.errors(), Vec::<String>::new());
}
