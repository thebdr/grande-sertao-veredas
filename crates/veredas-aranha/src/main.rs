//! `veredas-aranha`, the scanner from the command line (`docs/aranha-plan.md`, "Scope", the
//! executable; phase 7): listen on interfaces for a while and write the scan; identify; diff two
//! scans. Plain text out, for the engineer and for a script: a summary of what was found, the
//! answers, the diff. The table, the topology and the diff drawn are Oficina's (Q280).
//!
//! ```text
//! veredas-aranha listen <interface>... [--for <seconds>] [--identify] [--ports] --out <scan.toml>
//! veredas-aranha identify <interface> [--name <station> | --alias <alias>] [--wait <ms>]
//! veredas-aranha diff <baseline.toml> <scan.toml>
//! ```
//!
//! Exit status: 0 done (for `diff`, no change), 1 for `diff` when something changed, 2 for an error
//! (a usage refused, a file not read, an interface not opened), its reason on stderr.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use veredas_aranha::diff::{self, Diff};
use veredas_aranha::scan::{Capture, Scan};

const USAGE: &str = "usage:
  veredas-aranha listen <interface>... [--for <seconds>] [--identify] [--ports] --out <scan.toml>
  veredas-aranha identify <interface> [--name <station> | --alias <alias>] [--wait <ms>]
  veredas-aranha diff <baseline.toml> <scan.toml>";

/// The longest listening asked for: an hour.
const MAX_FOR: u64 = 3600;

/// A refusal, said on stderr, exit status 2.
struct Refused(String);

impl<E: std::fmt::Display> From<E> for Refused {
    fn from(e: E) -> Refused {
        Refused(e.to_string())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let run = match args.first().map(String::as_str) {
        Some("--help" | "-h" | "help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some("listen") => listen(&args[1..]).map(|()| ExitCode::SUCCESS),
        Some("identify") => identify(&args[1..]).map(|()| ExitCode::SUCCESS),
        Some("diff") => diff_files(&args[1..]),
        Some(other) => Err(Refused(format!("no command {other:?}\n{USAGE}"))),
        None => Err(Refused(USAGE.to_string())),
    };
    match run {
        Ok(code) => code,
        Err(Refused(why)) => {
            eprintln!("veredas-aranha: {why}");
            ExitCode::from(2)
        }
    }
}

/// The value after a flag.
fn value<'a>(args: &'a [String], i: &mut usize, flag: &str) -> Result<&'a str, Refused> {
    *i += 1;
    args.get(*i).map(String::as_str).ok_or_else(|| Refused(format!("{flag} needs a value\n{USAGE}")))
}

fn diff_files(args: &[String]) -> Result<ExitCode, Refused> {
    let [baseline, scan] = args else {
        return Err(Refused(format!("diff takes two scan files\n{USAGE}")));
    };
    let read = |p: &String| Scan::read_file(Path::new(p)).map_err(|e| Refused(format!("{p}: {e}")));
    let (b, s) = (read(baseline)?, read(scan)?);
    let d = diff::diff(&b, &s)?;
    for line in lines(&d) {
        println!("{line}");
    }
    Ok(if d.is_empty() { ExitCode::SUCCESS } else { ExitCode::from(1) })
}

/// The diff as lines, one a change, in the diff's order; "no change" when there is none.
fn lines(d: &Diff) -> Vec<String> {
    let mut out = Vec::new();
    for x in &d.appeared {
        out.push(format!("appeared {} {:?}", x.mac, x.label));
    }
    for x in &d.vanished {
        out.push(format!("vanished {} {:?}", x.mac, x.label));
    }
    let said = |w: diff::Said| match w {
        diff::Said::StationName => "station name",
        diff::Said::SystemName => "system name",
        diff::Said::IpSuite => "ip suite",
        diff::Said::ArpAddress => "arp address",
        diff::Said::ManagementAddresses => "management addresses",
    };
    for c in &d.renamed {
        out.push(format!("renamed {} {}: {:?} -> {:?}", c.mac, said(c.what), c.from, c.to));
    }
    for c in &d.readdressed {
        out.push(format!("readdressed {} {}: {:?} -> {:?}", c.mac, said(c.what), c.from, c.to));
    }
    let or_none = |v: &Option<String>| v.as_ref().map_or("none".to_string(), |v| format!("{v:?}"));
    for o in &d.one_sided {
        out.push(format!("one-sided {} {}: baseline {}, scan {}", o.mac, said(o.what), or_none(&o.baseline), or_none(&o.scan)));
    }
    let end = |e: &veredas_aranha::topology::End| if e.port.is_empty() { e.node.clone() } else { format!("{} {}", e.node, e.port) };
    for l in &d.links_added {
        out.push(format!("link added {} = {} ({})", end(&l.a), end(&l.b), spelled(l.confidence)));
    }
    for l in &d.links_removed {
        out.push(format!("link removed {} = {} ({})", end(&l.a), end(&l.b), spelled(l.confidence)));
    }
    let macs = |c: &veredas_aranha::fusion::Conflict| c.macs.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(", ");
    for c in &d.conflicts_new {
        out.push(format!("conflict new {} {:?}: {}", spelled(c.kind), c.value, macs(c)));
    }
    for c in &d.conflicts_cleared {
        out.push(format!("conflict cleared {} {:?}: {}", spelled(c.kind), c.value, macs(c)));
    }
    for i in &d.interfaces_added {
        out.push(format!("interface added {i}"));
    }
    for i in &d.interfaces_removed {
        out.push(format!("interface removed {i}"));
    }
    if out.is_empty() {
        out.push("no change".to_string());
    }
    out
}

/// A kind as the scan file spells it (`duplicate-name`, `heard`).
fn spelled<T: serde::Serialize>(v: T) -> String {
    toml::Value::try_from(v).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn open(iface: &str) -> Result<Box<dyn veredas_aranha::wire::Wire>, Refused> {
    use veredas_aranha::wire::raw::{Options, RawLink};
    let link = RawLink::open_with(iface, Options { ethertype: None, promiscuous: false, outgoing: false }).map_err(|e| Refused(format!("{iface}: {e}")))?;
    Ok(Box::new(link))
}

#[cfg(not(target_os = "linux"))]
fn open(_iface: &str) -> Result<Box<dyn veredas_aranha::wire::Wire>, Refused> {
    Err(Refused("raw frames are read on Linux alone for now (docs/questions.md Q281)".to_string()))
}

fn identify(args: &[String]) -> Result<(), Refused> {
    use veredas_aranha::ask::{self, Identify};
    use veredas_aranha::listen::{Listener, Options};
    use veredas_aranha::profinet::dcp::Filter;
    let mut iface = None;
    let mut ask = Identify::default();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--name" => ask.filter = Filter::Name(value(args, &mut i, "--name")?.to_string()),
            "--alias" => ask.filter = Filter::Alias(value(args, &mut i, "--alias")?.to_string()),
            "--wait" => ask.wait = Duration::from_millis(value(args, &mut i, "--wait")?.parse().map_err(|_| Refused(format!("--wait takes milliseconds\n{USAGE}")))?),
            flag if flag.starts_with("--") => return Err(Refused(format!("identify takes no {flag}\n{USAGE}"))),
            name if iface.is_none() => iface = Some(name.to_string()),
            _ => return Err(Refused(format!("identify asks on one interface\n{USAGE}"))),
        }
        i += 1;
    }
    let iface = iface.ok_or_else(|| Refused(format!("identify needs an interface\n{USAGE}")))?;
    let l = Listener::start(vec![open(&iface)?], Options::default())?;
    let asked = ask::identify(&l, &iface, &ask);
    let s = l.stop();
    let asked = asked?;
    for a in &asked.answers {
        let d = s.fusion.devices.get(&a.mac);
        let ip = a.identity.ip.as_ref().map_or("none".to_string(), |s| s.ip.to_string());
        let ids = a.identity.vendor_id.zip(a.identity.device_id).map_or("none".to_string(), |(v, d)| format!("{v:#06x}/{d:#06x}"));
        println!("{} {:?} {} ids {} frame {}{}", a.mac, a.identity.name.clone().unwrap_or_default(), ip, ids, a.frame, d.map_or(String::new(), |d| format!(" ({})", d.label())));
    }
    println!("{} answer(s) on {iface}, {} other(s), {} lost", asked.answers.len(), asked.others, asked.lost);
    Ok(())
}

fn listen(args: &[String]) -> Result<(), Refused> {
    use veredas_aranha::listen::{Listener, Options};
    use veredas_aranha::topology::{self, Interface};
    use veredas_aranha::{ask, read};
    let mut ifaces: Vec<String> = Vec::new();
    let (mut seconds, mut asks, mut ports, mut out) = (10u64, false, false, None::<PathBuf>);
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--for" => {
                seconds = value(args, &mut i, "--for")?.parse().map_err(|_| Refused(format!("--for takes whole seconds\n{USAGE}")))?;
                if seconds == 0 || seconds > MAX_FOR {
                    return Err(Refused(format!("--for takes 1 to {MAX_FOR} seconds")));
                }
            }
            "--identify" => asks = true,
            "--ports" => ports = true,
            "--out" => out = Some(PathBuf::from(value(args, &mut i, "--out")?)),
            flag if flag.starts_with("--") => return Err(Refused(format!("listen takes no {flag}\n{USAGE}"))),
            name => {
                if ifaces.iter().any(|n| n == name) {
                    return Err(Refused(format!("{name} is named twice")));
                }
                ifaces.push(name.to_string());
            }
        }
        i += 1;
    }
    let out = out.ok_or_else(|| Refused(format!("listen writes its scan to --out <scan.toml>\n{USAGE}")))?;
    if ifaces.is_empty() {
        return Err(Refused(format!("listen needs an interface\n{USAGE}")));
    }
    if out.extension().is_none_or(|e| e != "toml") {
        return Err(Refused(format!("{}: a scan file is .toml", out.display())));
    }
    let wires = ifaces.iter().map(|n| open(n)).collect::<Result<Vec<_>, _>>()?;
    let l = Listener::start(wires, Options::default())?;
    // Listening first; an Identify on each interface a second in, so the wire's own traffic is
    // heard before anything is asked.
    let started = std::time::Instant::now();
    let total = Duration::from_secs(seconds);
    if asks {
        std::thread::sleep(Duration::from_secs(1).min(total));
        for iface in &ifaces {
            let asked = ask::identify(&l, iface, &ask::Identify::default())?;
            eprintln!("{iface}: {} device(s) answered the Identify", asked.answers.len());
        }
    }
    std::thread::sleep(total.saturating_sub(started.elapsed()));
    // Each device's ports read on the interface it was heard on, one device at a time.
    let mut neighbours = Vec::new();
    if ports {
        let s = l.snapshot();
        for d in s.fusion.devices.values() {
            let Some(iface) = ifaces.iter().find(|i| d.heard_on.contains(*i)) else { continue };
            if d.ip.current().is_none_or(|s| s.ip.is_unspecified()) {
                continue;
            }
            match read::ports(&l, iface, d, &read::Options::default()) {
                Ok(p) => neighbours.extend(p.neighbours),
                Err((subslot, e)) => eprintln!("{} {}: subslot {subslot:#06x} not read: {e}", d.mac, d.label()),
            }
        }
    }
    let s = l.snapshot();
    let (_, frames) = l.read_from(s.kept.0);
    l.stop();
    let interfaces: Vec<Interface> = s.stats.interfaces.iter().map(|(name, i)| Interface { name: name.clone(), mac: i.mac }).collect();
    let t = topology::build(&s.fusion, &interfaces, &neighbours);
    let capture_path = out.with_extension("pcap");
    let capture = Capture {
        file: capture_path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        first_frame: frames.first().map_or(s.kept.1, |(i, _)| *i),
        frames: frames.len() as u64,
    };
    let kept: Vec<veredas_aranha::wire::Frame> = frames.into_iter().map(|(_, k)| k.frame).collect();
    veredas_aranha::wire::pcap::write_file(&capture_path, &kept).map_err(|e| Refused(format!("{}: {e}", capture_path.display())))?;
    let scan = Scan::new(&s.fusion, &s.stats, &t, veredas_aranha::wire::stamp(std::time::SystemTime::now()), Some(capture));
    scan.write_file(&out)?;
    for d in &scan.devices {
        let ip = d.ip.current().map_or("none".to_string(), |s| s.ip.to_string());
        let on: Vec<&str> = d.heard_on.iter().map(String::as_str).collect();
        println!("{} {:?} {} on {}", d.mac, d.label(), ip, on.join(","));
    }
    for c in &scan.conflicts {
        println!("conflict {} {:?}: {}", spelled(c.kind), c.value, c.macs.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(", "));
    }
    println!("{} device(s), {} link(s), {} frame(s) heard; {} and {}", scan.devices.len(), scan.topology.links.len(), scan.stats.heard, out.display(), capture_path.display());
    Ok(())
}
