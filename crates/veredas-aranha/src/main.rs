//! `veredas-aranha`, the scanner from the command line (`docs/aranha-plan.md`, "Scope", the
//! executable; phase 7): listen on interfaces for a while and write the scan; identify; diff two
//! scans. Plain text out, for the engineer and for a script: a summary of what was found, the
//! answers, the diff. The table, the topology and the diff drawn are Oficina's (Q280).
//!
//! ```text
//! veredas-aranha listen <interface>... [--for <seconds>] [--identify] [--ports] --out <scan.toml>
//! veredas-aranha identify <interface> [--name <station> | --alias <alias>] [--wait <ms>]
//! veredas-aranha diff <baseline.toml> <scan.toml>
//! veredas-aranha --version | --help
//! ```
//!
//! Exit status: 0 done (for `diff`, no change), 1 for `diff` when something changed, 2 for an error
//! (a usage refused, a file not read, an interface not opened), its reason on stderr. Arguments are
//! read as the system gives them (a path need not be UTF-8); a stdout or stderr closed or no longer
//! read changes neither what is done nor the exit status (C-033's rounds 5 and 6 found both in the
//! bench's executable).

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use veredas_aranha::diff::{self, Diff};
use veredas_aranha::scan::{Capture, Scan};

const USAGE: &str = "usage:
  veredas-aranha listen <interface>... [--for <seconds>] [--identify] [--ports] --out <scan.toml>
  veredas-aranha identify <interface> [--name <station> | --alias <alias>] [--wait <ms>]
  veredas-aranha diff <baseline.toml> <scan.toml>
  veredas-aranha --version | --help";

/// The longest listening asked for: an hour.
const MAX_FOR: u64 = 3600;

/// A refusal, said on stderr, exit status 2.
struct Refused(String);

impl<E: std::fmt::Display> From<E> for Refused {
    fn from(e: E) -> Refused {
        Refused(e.to_string())
    }
}

/// A line on stdout, a stdout closed or failing ignored.
fn say(line: &str) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

/// A line on stderr, a stderr closed or failing ignored: the exit status stays the one stated.
fn complain(line: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let first = args.first().map(|a| a.to_str());
    let run = match first {
        Some(Some("--help" | "-h" | "help")) => {
            say(USAGE);
            return ExitCode::SUCCESS;
        }
        Some(Some("--version")) => {
            say(&format!("veredas-aranha {}", env!("CARGO_PKG_VERSION")));
            return ExitCode::SUCCESS;
        }
        Some(Some("listen")) => listen(&args[1..]).map(|()| ExitCode::SUCCESS),
        Some(Some("identify")) => identify(&args[1..]).map(|()| ExitCode::SUCCESS),
        Some(Some("diff")) => diff_files(&args[1..]),
        Some(_) => Err(Refused(format!("no command {:?}\n{USAGE}", args[0]))),
        None => Err(Refused(USAGE.to_string())),
    };
    match run {
        Ok(code) => code,
        Err(Refused(why)) => {
            complain(&format!("veredas-aranha: {why}"));
            ExitCode::from(2)
        }
    }
}

/// An argument as text: a flag, a number, a name. Only a path may be other than UTF-8.
fn text<'a>(arg: &'a OsStr, what: &str) -> Result<&'a str, Refused> {
    arg.to_str().ok_or_else(|| Refused(format!("{what} {arg:?} is not UTF-8")))
}

/// The value after a flag.
fn value<'a>(args: &'a [OsString], i: &mut usize, flag: &str) -> Result<&'a OsStr, Refused> {
    *i += 1;
    args.get(*i).map(OsString::as_os_str).ok_or_else(|| Refused(format!("{flag} needs a value\n{USAGE}")))
}

/// An option given once: a second is refused, never taken over the first.
fn once<T>(slot: &mut Option<T>, flag: &str, v: T) -> Result<(), Refused> {
    if slot.is_some() {
        return Err(Refused(format!("{flag} is given twice\n{USAGE}")));
    }
    *slot = Some(v);
    Ok(())
}

fn diff_files(args: &[OsString]) -> Result<ExitCode, Refused> {
    let [baseline, scan] = args else {
        return Err(Refused(format!("diff takes two scan files\n{USAGE}")));
    };
    let read = |p: &OsString| Scan::read_file(Path::new(p)).map_err(|e| Refused(format!("{}: {e}", Path::new(p).display())));
    let (b, s) = (read(baseline)?, read(scan)?);
    let d = diff::diff(&b, &s)?;
    for line in lines(&d) {
        say(&line);
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
    // A node and its port quoted: a port or a chassis is a device's own text, which may hold
    // anything, a line break among it.
    let end = |e: &veredas_aranha::topology::End| if e.port.is_empty() { format!("{:?}", e.node) } else { format!("{:?} {:?}", e.node, e.port) };
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

/// The scan and its capture written under names of their own beside them, then put in place, the
/// capture first: a failure before that leaves what was there as it was.
fn put_in_place(out: &Path, capture: &Path, scan: &Scan, frames: &[veredas_aranha::wire::Frame]) -> Result<(), Refused> {
    let partial = |p: &Path| {
        let mut name = OsString::from(".");
        name.push(p.file_name().unwrap_or_default());
        name.push(".partial");
        p.with_file_name(name)
    };
    let (scan_tmp, capture_tmp) = (partial(out), partial(capture));
    let written = (|| -> Result<(), Refused> {
        let text = scan.to_toml()?;
        std::fs::write(&scan_tmp, text).map_err(|e| Refused(format!("{}: {e}", out.display())))?;
        veredas_aranha::wire::pcap::write_file(&capture_tmp, frames).map_err(|e| Refused(format!("{}: {e}", capture.display())))?;
        std::fs::rename(&capture_tmp, capture).map_err(|e| Refused(format!("{}: {e}", capture.display())))?;
        std::fs::rename(&scan_tmp, out).map_err(|e| Refused(format!("{}: {e}", out.display())))
    })();
    let _ = std::fs::remove_file(&scan_tmp);
    let _ = std::fs::remove_file(&capture_tmp);
    written
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

fn identify(args: &[OsString]) -> Result<(), Refused> {
    use veredas_aranha::ask::{self, Identify};
    use veredas_aranha::listen::{Listener, Options};
    use veredas_aranha::profinet::dcp::Filter;
    let mut iface = None;
    let mut ask = Identify::default();
    let (mut filter, mut wait) = (None, None);
    let mut i = 0;
    while i < args.len() {
        match text(&args[i], "an argument")? {
            "--name" => once(&mut filter, "--name or --alias", Filter::Name(text(value(args, &mut i, "--name")?, "a station name")?.to_string()))?,
            "--alias" => once(&mut filter, "--name or --alias", Filter::Alias(text(value(args, &mut i, "--alias")?, "an alias")?.to_string()))?,
            "--wait" => {
                let ms: u64 = text(value(args, &mut i, "--wait")?, "--wait")?.parse().map_err(|_| Refused(format!("--wait takes milliseconds\n{USAGE}")))?;
                once(&mut wait, "--wait", Duration::from_millis(ms))?;
            }
            flag if flag.starts_with("--") => return Err(Refused(format!("identify takes no {flag}\n{USAGE}"))),
            name if iface.is_none() => iface = Some(name.to_string()),
            _ => return Err(Refused(format!("identify asks on one interface\n{USAGE}"))),
        }
        i += 1;
    }
    if let Some(f) = filter {
        ask.filter = f;
    }
    if let Some(w) = wait {
        ask.wait = w;
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
        say(&format!("{} {:?} {} ids {}{}", a.mac, a.identity.name.clone().unwrap_or_default(), ip, ids, d.map_or(String::new(), |d| format!(" ({:?})", d.label()))));
    }
    say(&format!("{} answer(s) on {iface}, {} other(s), {} lost", asked.answers.len(), asked.others, asked.lost));
    Ok(())
}

fn listen(args: &[OsString]) -> Result<(), Refused> {
    use veredas_aranha::listen::{Listener, Options};
    use veredas_aranha::topology::{self, Interface};
    use veredas_aranha::{ask, read};
    let mut ifaces: Vec<String> = Vec::new();
    let (mut seconds, mut asks, mut ports, mut out) = (None::<u64>, None, None, None::<PathBuf>);
    let mut i = 0;
    while i < args.len() {
        // (A flag's value is taken by `value` with its flag, so a path after `--out` is never read
        // as text here.)
        match text(&args[i], "an argument")? {
            "--for" => {
                let n: u64 = text(value(args, &mut i, "--for")?, "--for")?.parse().map_err(|_| Refused(format!("--for takes whole seconds\n{USAGE}")))?;
                if n == 0 || n > MAX_FOR {
                    return Err(Refused(format!("--for takes 1 to {MAX_FOR} seconds")));
                }
                once(&mut seconds, "--for", n)?;
            }
            "--identify" => once(&mut asks, "--identify", true)?,
            "--ports" => once(&mut ports, "--ports", true)?,
            "--out" => once(&mut out, "--out", PathBuf::from(value(args, &mut i, "--out")?))?,
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
    let (seconds, asks, ports) = (seconds.unwrap_or(10), asks.unwrap_or(false), ports.unwrap_or(false));
    let out = out.ok_or_else(|| Refused(format!("listen writes its scan to --out <scan.toml>\n{USAGE}")))?;
    if ifaces.is_empty() {
        return Err(Refused(format!("listen needs an interface\n{USAGE}")));
    }
    if out.extension().is_none_or(|e| e != "toml") {
        return Err(Refused(format!("{}: a scan file is .toml", out.display())));
    }
    // The scan names its capture, so its file name is text the scan can hold; both are checked
    // before anything is opened, and an existing one is replaced only once both are written.
    if out.file_name().and_then(|n| n.to_str()).is_none() {
        return Err(Refused(format!("{}: a scan file's name is UTF-8, as the scan names its capture by it", out.display())));
    }
    let capture_path = out.with_extension("pcap");
    for p in [&out, &capture_path] {
        if p.exists() {
            if !p.is_file() {
                return Err(Refused(format!("{}: there is something there that is no file", p.display())));
            }
            std::fs::OpenOptions::new().write(true).open(p).map_err(|e| Refused(format!("{}: {e}", p.display())))?;
        }
    }
    let wires = ifaces.iter().map(|n| open(n)).collect::<Result<Vec<_>, _>>()?;
    let l = Listener::start(wires, Options::default())?;
    // Listening first; an Identify on every interface at once a second in, so the wire's own
    // traffic is heard before anything is asked. Its wait may carry the listening past the time
    // given.
    let started = std::time::Instant::now();
    let total = Duration::from_secs(seconds);
    if asks {
        std::thread::sleep(Duration::from_secs(1).min(total));
        let asked: Vec<_> = std::thread::scope(|scope| {
            let each: Vec<_> = ifaces.iter().map(|iface| scope.spawn(|| ask::identify(&l, iface, &ask::Identify::default()))).collect();
            each.into_iter().map(|h| h.join()).collect()
        });
        for (iface, a) in ifaces.iter().zip(asked) {
            let a = a.map_err(|_| Refused(format!("{iface}: the Identify's thread panicked")))??;
            complain(&format!("{iface}: {} device(s) answered the Identify", a.answers.len()));
        }
    }
    std::thread::sleep(total.saturating_sub(started.elapsed()));
    // Each device's ports read on the interface it was heard on, one device at a time; what was
    // read of a device before a read failed is kept.
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
                Err(cut) => {
                    neighbours.extend(cut.read.neighbours);
                    complain(&format!("{} {:?}: subslot {:#06x} not read: {}", d.mac, d.label(), cut.subslot, cut.error));
                }
            }
        }
    }
    let s = l.snapshot();
    let (_, frames) = l.read_from(s.kept.0);
    l.stop();
    let interfaces: Vec<Interface> = s.stats.interfaces.iter().map(|(name, i)| Interface { name: name.clone(), mac: i.mac }).collect();
    let t = topology::build(&s.fusion, &interfaces, &neighbours);
    let capture = Capture {
        file: capture_path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string(),
        first_frame: frames.first().map_or(s.kept.1, |(i, _)| *i),
        frames: frames.len() as u64,
    };
    let kept: Vec<veredas_aranha::wire::Frame> = frames.into_iter().map(|(_, k)| k.frame).collect();
    let scan = Scan::new(&s.fusion, &s.stats, &t, veredas_aranha::wire::stamp(std::time::SystemTime::now()), Some(capture));
    put_in_place(&out, &capture_path, &scan, &kept)?;
    for d in &scan.devices {
        let ip = d.ip.current().map_or("none".to_string(), |s| s.ip.to_string());
        let on: Vec<&str> = d.heard_on.iter().map(String::as_str).collect();
        say(&format!("{} {:?} {} on {}", d.mac, d.label(), ip, on.join(",")));
    }
    for c in &scan.conflicts {
        say(&format!("conflict {} {:?}: {}", spelled(c.kind), c.value, c.macs.iter().map(|m| m.to_string()).collect::<Vec<_>>().join(", ")));
    }
    say(&format!("{} device(s), {} link(s), {} frame(s) heard; {} and {}", scan.devices.len(), scan.topology.links.len(), scan.stats.heard, out.display(), capture_path.display()));
    Ok(())
}
