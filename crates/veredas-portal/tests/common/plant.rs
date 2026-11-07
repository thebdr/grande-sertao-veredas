//! The plant the session and the server meet: a stamped log the test writes, the bench's OPC UA
//! server and the controller reference's machine, from one bench file; and the session that
//! reads the three.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::NaiveDate;
use veredas_core::telegram::{Role, catalog, schema};
use veredas_maquinar::bench::{self, Bench};
use veredas_portal::channel::ChannelConfig;
use veredas_portal::opc::{OpcRuleConfig, OpcSourceConfig, OpcTagConfig};
use veredas_portal::session::{CaptureRuleConfig, LogKind, LogSourceConfig, SessionConfig, TimestampConfig};

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

fn doc(name: &str) -> String {
    let doc = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs").join(name);
    std::fs::read_to_string(doc).unwrap().replace("\r\n", "\n")
}

/// The TOML blocks of a reference, in order.
fn blocks(name: &str) -> Vec<String> {
    let text = doc(name);
    let mut out = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("```toml\n") {
        let body = &rest[start + "```toml\n".len()..];
        let end = body.find("```").unwrap();
        out.push(body[..end].to_string());
        rest = &body[end + 3..];
    }
    out
}

pub fn channel_text() -> String {
    blocks("channel-schema.md")[0].clone()
}

pub fn catalog_text() -> String {
    blocks("controller-schema.md")[0].clone()
}

pub fn machine_text() -> String {
    blocks("controller-schema.md")[1].clone()
}

pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

/// The bench: the OPC UA server `plc` on a free port (a drawn speed, a run flag, a packed word,
/// a label, a write-only command) and the machine `machine` of the controller reference,
/// listening on `port`.
pub fn plant(dir: &Path, port: u16) -> Bench {
    std::fs::write(dir.join("channel.toml"), channel_text()).unwrap();
    std::fs::write(dir.join("catalog.toml"), catalog_text()).unwrap();
    std::fs::write(dir.join("machine.toml"), machine_text()).unwrap();
    let text = format!(
        "format = \"veredas-bench\"\nversion = 1\nseed = 3\n\n[[opc]]\nname = \"plc\"\nport = 0\ntags = [\n  {{ id = \"speed\", type = \"float\", value = \"1.5\", randomize = {{ low = 1.0, high = 2.0, decimals = 2, interval_ms = 100 }} }},\n  {{ id = \"run\", type = \"bool\", access = \"read-write\" }},\n  {{ id = \"word\", type = \"int\", value = \"5\", access = \"read-write\" }},\n  {{ id = \"label\", type = \"text\", value = \"idle\", access = \"read-write\" }},\n  {{ id = \"cmd\", type = \"int\", access = \"write\" }},\n]\n\n[[channel]]\nname = \"machine\"\nrole = \"listen\"\nport = {port}\nchannel = \"channel.toml\"\ncatalog = \"catalog.toml\"\ncontroller = \"machine.toml\"\n"
    );
    std::fs::write(dir.join("bench.toml"), &text).unwrap();
    Bench::start(bench::load(&text, dir).unwrap()).unwrap()
}

/// The machine's controller, as the bench reports it.
pub fn machine_status(bench: &Bench) -> veredas_maquinar::controller::ControllerStatus {
    bench.channel("machine").unwrap().status().controller.unwrap()
}

/// The stamp every test line carries, and the index it lands on: 2026-07-19, ten o'clock, plus
/// `i` seconds, read as UTC.
pub fn stamp_ms(i: u32) -> i64 {
    NaiveDate::from_ymd_opt(2026, 7, 19).unwrap().and_hms_opt(10, 0, 0).unwrap().and_utc().timestamp_millis() + i as i64 * 1000
}

/// The temperature line `i` carries.
pub fn temp(i: u32) -> f64 {
    20.0 + i as f64 * 0.5
}

/// A stamped line the readings rule captures: a temperature, an item (one of three) and a state.
pub fn stamped_line(i: u32) -> String {
    format!("2026-07-19 10:{:02}:{:02} temp={:.1} item=A{} state=run\n", i / 60, i % 60, temp(i), i % 3)
}

pub fn write_lines(path: &Path, range: std::ops::Range<u32>) {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
    for i in range {
        f.write_all(stamped_line(i).as_bytes()).unwrap();
    }
    f.flush().unwrap();
}

pub fn log_source(id: &str, path: &Path) -> LogSourceConfig {
    LogSourceConfig {
        id: id.into(),
        display_name: "the plant log".into(),
        kind: LogKind::File { path: path.to_string_lossy().into_owned() },
        timestamp: Some(TimestampConfig {
            pattern: r"^(?P<ts>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2})".into(),
            formats: vec!["%Y-%m-%d %H:%M:%S".into()],
        }),
    }
}

/// The readings rule: a float temperature keyed by the item, the state as text.
pub fn readings_rule(scope: Option<&str>) -> CaptureRuleConfig {
    CaptureRuleConfig {
        name: "readings".into(),
        pattern: r"temp=(?P<temp>[0-9.]+) item=(?P<item>\w+) state=(?P<state>\w+)".into(),
        scope: scope.map(String::from),
        types: BTreeMap::from([("temp".to_string(), "float".to_string())]),
        key: Some("item".into()),
    }
}

fn tag(node: &str, series: &str, ty: &str, writable: bool) -> OpcTagConfig {
    OpcTagConfig { node_id: node.into(), series: series.into(), ty: ty.into(), writable, bits: vec![] }
}

/// The node id of a plant tag, as the server names it.
pub fn node(bench: &Bench, id: &str) -> String {
    bench.server("plc").unwrap().node_id_str(id)
}

/// The OPC source `plc`: one rule `plc_main` polling the speed, the run flag and the word every
/// 50 ms, with the word and the command writable under their own names.
pub fn opc_source(bench: &Bench) -> OpcSourceConfig {
    OpcSourceConfig {
        source_id: "plc".into(),
        display_name: "the plc".into(),
        endpoint_url: bench.server("plc").unwrap().endpoint_url.clone(),
        user: None,
        password_env: None,
        poll_ms: 50,
        rules: vec![OpcRuleConfig {
            name: "plc_main".into(),
            store_on_change: false,
            tags: vec![
                tag(&node(bench, "speed"), "speed", "float", false),
                tag(&node(bench, "run"), "run", "int", false),
                tag(&node(bench, "word"), "word", "int", false),
                tag(&node(bench, "word"), "word_cmd", "int", true),
                tag(&node(bench, "cmd"), "cmd", "int", true),
            ],
        }],
    }
}

/// The channel `sorter`, dialling the machine, with no reply rules.
pub fn channel_config(port: u16) -> ChannelConfig {
    ChannelConfig {
        source_id: "sorter".into(),
        display_name: "the sorter".into(),
        role: Role::Connect { host: "127.0.0.1".into(), port },
        spec: schema::load(&channel_text()).unwrap(),
        catalog: catalog::load(&catalog_text()).unwrap(),
        replies: vec![],
    }
}

/// The stop command the catalog writes: code 0.
pub fn stop_command() -> HashMap<String, String> {
    HashMap::from([("code".to_string(), "0".to_string())])
}

/// A session over the three: the log at `log`, the plant's server and its machine.
pub fn session_config(name: &str, log: &Path, bench: &Bench, port: u16) -> SessionConfig {
    let mut cfg = SessionConfig::new(name);
    cfg.logs.push(log_source("log", log));
    cfg.rules.push(readings_rule(None));
    cfg.opc.push(opc_source(bench));
    cfg.channels.push(channel_config(port));
    cfg
}
