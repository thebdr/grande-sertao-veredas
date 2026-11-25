//! The cut's run of the bundle (docs/alforge-plan.md, "How a release is cut", step 5): from an
//! extracted bundle, the bench started on `planta/bench.toml`, then supervisão on
//! `planta/supervisao.toml`, which starts the plant's portal session in its own process; the plant
//! read over HTTP as a browser reads it; both stopped by closing their stdin. The plant is copied
//! and only its ports change (the bench's OPC UA port, the project's endpoint, supervisão's
//! address), to free ones on the loopback, so the run never meets another on this machine.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::vm::{self, VmError};

/// An HTTP answer: its status and its body.
#[derive(Debug, Clone)]
pub struct Answer {
    pub status: u16,
    pub body: String,
}

fn dechunk(body: &str) -> String {
    let mut out = String::new();
    let mut rest = body;
    while let Some((size, after)) = rest.split_once("\r\n") {
        let Ok(n) = usize::from_str_radix(size.trim(), 16) else { break };
        if n == 0 || after.len() < n {
            break;
        }
        out.push_str(&after[..n]);
        rest = after[n..].trim_start_matches("\r\n");
    }
    out
}

/// One HTTP/1.1 request, the connection closed after it; `None` when nothing answered.
pub fn http(addr: SocketAddr, method: &str, path: &str, body: Option<&str>) -> Option<Answer> {
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(5)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(10))).ok()?;
    let b = body.unwrap_or("");
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{b}",
        b.len()
    );
    s.write_all(req.as_bytes()).ok()?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).ok()?;
    let text = String::from_utf8_lossy(&raw).into_owned();
    let (head, rest) = text.split_once("\r\n\r\n")?;
    let status = head.split_whitespace().nth(1)?.parse().ok()?;
    let chunked = head.lines().any(|l| l.to_ascii_lowercase().starts_with("transfer-encoding:") && l.to_ascii_lowercase().contains("chunked"));
    Some(Answer { status, body: if chunked { dechunk(rest) } else { rest.to_string() } })
}

/// What the run saw.
#[derive(Debug)]
pub struct Report {
    /// The page at `/`.
    pub page: Answer,
    /// Two snapshots of `/s/loop/objects.json`, taken `apart` from each other once every object
    /// had a value.
    pub first: Value,
    pub second: Value,
    pub apart: Duration,
    /// The binding's offered write posted to portal's surface under the session.
    pub write: Answer,
    /// The exit codes once stdin closed: the bench's, supervisão's.
    pub bench_exit: Option<i32>,
    pub supervisao_exit: Option<i32>,
    /// What each printed.
    pub bench_out: String,
    pub supervisao_out: String,
}

/// An object of a snapshot by id.
pub fn object<'a>(snapshot: &'a Value, id: &str) -> Option<&'a Value> {
    snapshot["objects"].as_array()?.iter().find(|o| o["id"] == id)
}

/// Why a report does not show the plant running: [`plant_faults`], and both executables ending
/// with 0. Empty when it does.
pub fn faults(r: &Report) -> Vec<String> {
    let mut out = plant_faults(r);
    if r.bench_exit != Some(0) {
        out.push(format!("the bench ended with {:?}: {}", r.bench_exit, r.bench_out));
    }
    if r.supervisao_exit != Some(0) {
        out.push(format!("supervisão ended with {:?}: {}", r.supervisao_exit, r.supervisao_out));
    }
    out
}

/// Why what was read over HTTP does not show the plant running: the page served; every object of
/// the loop carrying a value, not stale; both sources live; both parcels moved between the
/// snapshots; the write refused by the switch. The executables' exits are not read here (a
/// machine's services are not stopped to be judged).
pub fn plant_faults(r: &Report) -> Vec<String> {
    let mut out = Vec::new();
    if r.page.status != 200 || !r.page.body.contains("<html") {
        out.push(format!("the page answered {} without a page", r.page.status));
    }
    for id in ["speed", "read-rate", "infeed", "outlet-1", "outlet-2", "outlet-3", "parcel-1", "parcel-2"] {
        match object(&r.second, id) {
            Some(o) if o["value"].is_number() && o["stale"] == false => {}
            Some(o) => out.push(format!("object `{id}` shows no live value: {o}")),
            None => out.push(format!("object `{id}` is not in the feed")),
        }
    }
    for source in ["plc", "sorter"] {
        let state = &r.second["session"]["health"][source]["state"];
        if state != "live" {
            out.push(format!("source `{source}` is {state}, not live"));
        }
    }
    for id in ["parcel-1", "parcel-2"] {
        let a = object(&r.first, id).map(|o| o["place"].clone());
        let b = object(&r.second, id).map(|o| o["place"].clone());
        if a.is_none() || a == b || !b.as_ref().is_some_and(Value::is_number) {
            out.push(format!("`{id}` did not move along the loop in {} ms ({a:?} then {b:?})", r.apart.as_millis()));
        }
    }
    if r.write.status != 403 || !r.write.body.contains("writes are turned off") {
        out.push(format!("the offered write was not refused by the switch: {} {}", r.write.status, r.write.body));
    }
    out
}

/// Reads the plant served at `addr` as the cut's run does: waits within `within` until every
/// object carries a value, reads the feed again `apart` later, then the page and the offered
/// write. The exits are left unknown.
pub fn read_plant(addr: SocketAddr, within: Duration, apart: Duration) -> Report {
    let snapshot = || http(addr, "GET", "/s/loop/objects.json", None).and_then(|a| serde_json::from_str::<Value>(&a.body).ok());
    let all_valued = |s: &Value| s["objects"].as_array().is_some_and(|v| !v.is_empty() && v.iter().all(|o| o["value"].is_number()));
    let start = Instant::now();
    let mut first = Value::Null;
    while start.elapsed() < within {
        if let Some(s) = snapshot()
            && all_valued(&s)
        {
            first = s;
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    std::thread::sleep(apart);
    let second = snapshot().unwrap_or(Value::Null);
    let none = || Answer { status: 0, body: String::new() };
    let page = http(addr, "GET", "/", None).unwrap_or_else(none);
    let write = http(addr, "POST", "/s/loop/portal/api/opc/write", Some(r#"{"source":"plc","tag":"ns=2;s=release3","value":"1"}"#)).unwrap_or_else(none);
    Report { page, first, second, apart, write, bench_exit: None, supervisao_exit: None, bench_out: String::new(), supervisao_out: String::new() }
}

fn replace_once(path: &Path, from: &str, to: &str) -> Result<(), VmError> {
    let text = std::fs::read_to_string(path)?;
    if text.matches(from).count() != 1 {
        return Err(VmError::Failed { program: "the plant's copy".into(), status: "refused".into(), stderr: format!("{} does not hold `{from}` once", path.display()) });
    }
    std::fs::write(path, text.replacen(from, to, 1))?;
    Ok(())
}

fn spawn(bin: &Path, file: &str, dir: &Path) -> Result<Child, VmError> {
    Command::new(bin)
        .arg(file)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|source| VmError::Spawn { program: bin.display().to_string(), source })
}

fn finish(mut c: Child, within: Duration) -> (Option<i32>, String) {
    drop(c.stdin.take());
    let start = Instant::now();
    let code = loop {
        match c.try_wait() {
            Ok(Some(s)) => break s.code(),
            Ok(None) if start.elapsed() < within => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = c.kill();
                let _ = c.wait();
                break None;
            }
        }
    };
    let mut out = String::new();
    if let Some(mut o) = c.stdout.take() {
        let _ = o.read_to_string(&mut out);
    }
    if let Some(mut e) = c.stderr.take() {
        let _ = e.read_to_string(&mut out);
    }
    (code, out)
}

/// Runs the bundle extracted at `bundle` together, its plant copied into `work`.
pub fn run(bundle: &Path, work: &Path) -> Result<Report, VmError> {
    let plant = work.join("planta");
    let _ = std::fs::remove_dir_all(&plant);
    std::fs::create_dir_all(work)?;
    vm::run("cp", &["-r", &bundle.join("planta").display().to_string(), &plant.display().to_string()], None)?;
    let opc = vm::free_port()?;
    let web = vm::free_port()?;
    replace_once(&plant.join("bench.toml"), "port = 4855", &format!("port = {opc}"))?;
    replace_once(&plant.join("loop/project.toml"), "opc.tcp://127.0.0.1:4855/", &format!("opc.tcp://127.0.0.1:{opc}/"))?;
    replace_once(&plant.join("supervisao.toml"), "bind = \"0.0.0.0:8902\"", &format!("bind = \"127.0.0.1:{web}\""))?;
    let bin: PathBuf = bundle.join("bin");
    let bench = spawn(&bin.join("veredas-maquinar"), "bench.toml", &plant)?;
    std::thread::sleep(Duration::from_millis(500));
    let sup = spawn(&bin.join("veredas-supervisao"), "supervisao.toml", &plant)?;
    let addr: SocketAddr = ([127, 0, 0, 1], web).into();
    let read = read_plant(addr, Duration::from_secs(60), Duration::from_secs(2));
    let (supervisao_exit, supervisao_out) = finish(sup, Duration::from_secs(30));
    let (bench_exit, bench_out) = finish(bench, Duration::from_secs(30));
    Ok(Report { bench_exit, supervisao_exit, bench_out, supervisao_out, ..read })
}
