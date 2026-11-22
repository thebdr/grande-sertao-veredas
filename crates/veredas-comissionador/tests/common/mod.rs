//! The plant the engine's tests meet, through a portal session as comissionador's executable
//! starts one: a project file's text loaded by portal and started, over the bench's OPC UA server
//! and the controller reference's machine.
#![allow(dead_code)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use veredas_comissionador::Checked;
use veredas_comissionador::run::{Event, RunOptions, RunResult, run};
use veredas_comissionador::plant::LivePlant;
use veredas_maquinar::bench::{self, Bench};
use veredas_portal::session::{HealthState, Live, Session};

pub fn wait(within: Duration, mut what: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < within {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    what()
}

pub fn doc(name: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs").join(name);
    std::fs::read_to_string(p).unwrap().replace("\r\n", "\n")
}

/// The fenced blocks of a reference in a language, in order.
pub fn blocks(name: &str, lang: &str) -> Vec<String> {
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

/// The port a bench channel listens on, once it does.
pub fn listening_port(bench: &Bench, channel: &str) -> u16 {
    let started = Instant::now();
    loop {
        let wire = bench.channel(channel).unwrap().status().wire;
        if let Some(port) = wire.state.strip_prefix("listening on ").and_then(|s| s.rsplit_once(':')).and_then(|(_, p)| p.parse().ok()) {
            return port;
        }
        assert!(wire.state != "failed" && started.elapsed() < Duration::from_secs(10), "the bench's `{channel}` does not listen: {wire:?}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The bench: the OPC UA server `plc` (a speed set to 2.0 100 ms after `run` is written 1, a level
/// the test leaves alone) and the machine of the controller reference, listening on a port the
/// system hands it.
pub fn plant(dir: &Path) -> Bench {
    let toml = blocks("channel-schema.md", "toml");
    let controller = blocks("controller-schema.md", "toml");
    std::fs::write(dir.join("channel.toml"), &toml[0]).unwrap();
    std::fs::write(dir.join("catalog.toml"), &controller[0]).unwrap();
    std::fs::write(dir.join("machine.toml"), &controller[1]).unwrap();
    let text = "format = \"veredas-bench\"\nversion = 1\nseed = 3\n\n[[opc]]\nname = \"plc\"\nport = 0\ntags = [\n  { id = \"speed\", type = \"float\", value = \"1.5\" },\n  { id = \"run\", type = \"int\", value = \"0\", access = \"read-write\" },\n  { id = \"level\", type = \"float\", value = \"0.9\" },\n]\nreactions = [ { on_write = \"run\", equals = \"1\", tag = \"speed\", then = [ { after_ms = 100, set = \"2.0\" } ] } ]\n\n[[channel]]\nname = \"machine\"\nrole = \"listen\"\nport = 0\nchannel = \"channel.toml\"\ncatalog = \"catalog.toml\"\ncontroller = \"machine.toml\"\n";
    std::fs::write(dir.join("bench.toml"), text).unwrap();
    let bench = Bench::start(bench::load(text, dir).unwrap()).unwrap();
    listening_port(&bench, "machine");
    bench
}

/// The project portal runs: `the press hall`, its switch as `armed` says, the OPC UA source `plc`
/// polled every 50 ms into `plc_main` (`speed`, `run` writable, `level`), the channel `sorter`
/// dialling the machine, answering every announcement with destination `dest`, and the log
/// `plant.log` beside it, followed live, its `temp=` read into `readings`.
pub fn project_text(bench: &Bench, armed: bool, dest: u32) -> String {
    let plc = bench.server("plc").unwrap();
    let node = |id: &str| plc.node_id_str(id);
    format!(
        "format = \"veredas-project\"\nversion = 1\nname = \"the press hall\"\n\n[session]\nwrites_armed = {armed}\n\n[[opc]]\nsource_id = \"plc\"\nendpoint_url = \"{}\"\npoll_ms = 50\nrules = [ {{ name = \"plc_main\", tags = [\n  {{ node_id = \"{}\", series = \"speed\", type = \"float\" }},\n  {{ node_id = \"{}\", series = \"run\", type = \"int\", writable = true }},\n  {{ node_id = \"{}\", series = \"level\", type = \"float\" }},\n] }} ]\n\n[[channel]]\nsource_id = \"sorter\"\nrole = \"connect\"\nhost = \"127.0.0.1\"\nport = {}\nchannel = \"channel.toml\"\ncatalog = \"catalog.toml\"\nreplies = [ {{ on = \"announce\", send = \"answer\", fields = [ {{ name = \"item\", from = \"item\" }}, {{ name = \"dest\", value = {dest} }}, {{ name = \"flag\", value = 1 }} ] }} ]\n\n[[log]]\nid = \"plant\"\nfile = \"plant.log\"\ntimestamp = {{ pattern = \"^(?P<ts>\\\\S+ \\\\S+)\", formats = [\"%Y-%m-%d %H:%M:%S%.3f\"] }}\n\n[[rule]]\nname = \"readings\"\npattern = \"temp=(?P<temp>[0-9.]+)\"\nscope = \"plant\"\ntypes = {{ temp = \"float\" }}\n",
        plc.endpoint_url,
        node("speed"),
        node("run"),
        node("level"),
        listening_port(bench, "machine"),
    )
}

/// A session of the project, started as comissionador's executable starts one, once every source
/// reads live.
pub fn session(dir: &Path, text: &str) -> Session {
    std::fs::write(dir.join("plant.log"), "").unwrap();
    let project = veredas_portal::project::load(text, dir).unwrap();
    let s = Session::start(project.session_config(chrono::Local::now().naive_local())).unwrap();
    let live = s.live();
    // The log reads live once a line comes.
    std::thread::sleep(Duration::from_millis(200));
    log_line(dir, 0, "20.0");
    assert!(
        wait(Duration::from_secs(20), || live.health.read().unwrap().values().all(|h| h.state == HealthState::Live)),
        "the session's sources never read live: {:?}",
        live.health.read().unwrap()
    );
    s
}

/// A test's text checked, with the reference's function file beside it.
pub fn checked(text: &str) -> Checked {
    let functions = blocks("test-schema.md", "function").remove(0);
    veredas_comissionador::check_test(text, "t.test", &|p| {
        if p == "functions/routing.fn" { Ok(functions.clone()) } else { Err("not there".to_string()) }
    })
    .unwrap_or_else(|ps| panic!("{}", ps.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("\n")))
}

/// Run a test live against a session, every event kept.
pub fn live(c: &Checked, l: Arc<Live>) -> (RunResult, Vec<Event>) {
    let plant = LivePlant::new(l).unwrap();
    let mut events = Vec::new();
    let r = run(c, &plant, &RunOptions::live(), &mut |e| events.push(e.clone())).unwrap();
    (r, events)
}

/// Replay a test over a window of a session's store.
pub fn replay(c: &Checked, l: Arc<Live>, from: i64, to: i64) -> (RunResult, Vec<Event>) {
    let plant = LivePlant::new(l).unwrap();
    let mut events = Vec::new();
    let r = run(c, &plant, &RunOptions::replay(from, to), &mut |e| events.push(e.clone())).unwrap();
    (r, events)
}

pub const HEAD: &str = "format = \"veredas-test\"\nversion = 1\n\n";

/// A test of `body`, with `id T` first.
pub fn test_of(body: &str) -> String {
    format!("{HEAD}test \"t\" {{\n  id T\n{body}}}\n")
}

/// Run with options, every event kept, a refusal returned.
pub fn run_with(c: &Checked, l: Arc<Live>, opts: &RunOptions) -> Result<(RunResult, Vec<Event>), veredas_comissionador::run::Refused> {
    let plant = LivePlant::new(l).unwrap();
    let mut events = Vec::new();
    let r = run(c, &plant, opts, &mut |e| events.push(e.clone()))?;
    Ok((r, events))
}

/// The steps as `name #attempt outcome: detail`.
pub fn steps(r: &RunResult) -> Vec<String> {
    r.steps
        .iter()
        .map(|s| format!("{} #{} {}: {}", s.step, s.attempt, s.outcome.map(|o| o.to_string()).unwrap_or_else(|| "cut short".to_string()), s.detail))
        .collect()
}

/// A test's text checked with a function file of its own at `f.fn`.
pub fn checked_with(text: &str, functions: &str) -> Checked {
    let f = functions.to_string();
    veredas_comissionador::check_test(text, "t.test", &|p| if p == "f.fn" { Ok(f.clone()) } else { Err("not there".to_string()) })
        .unwrap_or_else(|ps| panic!("{}", ps.iter().map(|p| p.to_string()).collect::<Vec<_>>().join("\n")))
}

/// The bench's OPC UA tag, as text.
pub fn tag(bench: &Bench, id: &str) -> String {
    format!("{:?}", bench.server("plc").unwrap().get(id).unwrap())
}

/// A line of the plant's log, stamped `ago_ms` before the store's now, carrying `temp`.
pub fn log_line(dir: &Path, ago_ms: i64, temp: &str) {
    use std::io::Write;
    let at = chrono::DateTime::from_timestamp_millis(veredas_portal::serve::store_now_ms() - ago_ms).unwrap().naive_utc();
    let mut f = std::fs::OpenOptions::new().append(true).open(dir.join("plant.log")).unwrap();
    writeln!(f, "{} temp={temp}", at.format("%Y-%m-%d %H:%M:%S%.3f")).unwrap();
}
