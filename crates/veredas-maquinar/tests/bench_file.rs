//! The bench file (contract: the bench file clause): one versioned TOML file for the whole
//! plant, documented by a worked example this test loads from `docs/bench-schema.md` itself,
//! started with every part doing its work, and run by the executable; every key statement of
//! the reference held against the reader, and its text held as a copy.

use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use veredas_core::telegram::Role;
use veredas_maquinar::bench::*;
use veredas_maquinar::channel::CyclicMessage;
use veredas_maquinar::emit::{Encoding, ValueField};
use veredas_maquinar::ethernetip::{AssemblyConfig, ConnectionConfig, DataHeader, EnipConfig, Identity, PointConfig as EnipPoint, PointType as EnipType};
use veredas_maquinar::modbus::{ModbusConfig, PointConfig, PointType, Table, WordOrder};
use veredas_maquinar::opc::{Access, Credentials, Ramp, Randomize, Reaction, Step, TagConfig, TagType};

fn doc(name: &str) -> String {
    let doc = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs").join(name);
    std::fs::read_to_string(doc).unwrap().replace("\r\n", "\n")
}

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

fn doc_prose() -> String {
    doc("bench-schema.md").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn number_after(prose: &str, lead: &str) -> u64 {
    let at = prose.find(lead).unwrap_or_else(|| panic!("the prose no longer says `{lead}`"));
    let rest = prose[at + lead.len()..].trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let digits = digits.trim_end_matches('.');
    digits.parse::<f64>().unwrap_or_else(|_| panic!("no number after `{lead}`: {rest:.20}")) as u64
}

fn paragraphs(text: &str) -> Vec<String> {
    let mut text = text.replace("\r\n", "\n");
    while let Some(start) = text.find("```toml\n") {
        let end = start + text[start + 7..].find("```").unwrap() + 7 + 3;
        text.replace_range(start..end, "");
    }
    text.split("\n\n").map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|p| !p.is_empty()).collect()
}

fn documented_keys() -> Vec<(String, Vec<(String, String)>)> {
    let text = doc("bench-schema.md");
    let keys = &text[text.find("## Keys").unwrap()..];
    let mut out: Vec<(String, Vec<(String, String)>)> = Vec::new();
    for line in keys.lines() {
        if let Some(rest) = line.strip_prefix("**") {
            out.push((rest[..rest.find("**").unwrap()].to_string(), Vec::new()));
            continue;
        }
        if let Some(rest) = line.strip_prefix("- `") {
            // `low` (required), `high` (required): two keys on one line.
            let mut rest = rest;
            loop {
                let key = rest[..rest.find('`').unwrap()].to_string();
                let after = &rest[rest.find('`').unwrap() + 1..];
                let open = after.find('(').unwrap();
                let close = after.find(')').unwrap();
                out.last_mut().unwrap().1.push((key, after[open + 1..close].to_string()));
                match after[close..].strip_prefix("), `") {
                    Some(next) => rest = next,
                    None => break,
                }
            }
        }
    }
    out
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("veredas-bench-{tag}-{}-{}", std::process::id(), chrono::Local::now().format("%H%M%S%f")));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The worked example's folder: the bench file with the port the test chose, and beside it
/// the channel file, the catalog and the two rules files of the other references.
fn plant(dir: &Path) -> (PathBuf, u16) {
    let port = free_port();
    std::fs::write(dir.join("channel.toml"), blocks("channel-schema.md")[0].as_bytes()).unwrap();
    let controller = blocks("controller-schema.md");
    std::fs::write(dir.join("catalog.toml"), controller[0].as_bytes()).unwrap();
    std::fs::write(dir.join("machine.toml"), controller[1].as_bytes()).unwrap();
    std::fs::write(dir.join("unit.toml"), controller[2].as_bytes()).unwrap();
    let bench = blocks("bench-schema.md")[0].replace("port = 5020", &format!("port = {port}"));
    let path = dir.join("bench.toml");
    std::fs::write(&path, bench.as_bytes()).unwrap();
    (path, port)
}

fn wait(within: Duration, mut what: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < within {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    what()
}

fn refused(text: &str, base: &Path) -> String {
    match load(text, base) {
        Ok(_) => panic!("accepted: {text}"),
        Err(e) => e.to_string(),
    }
}

fn header() -> &'static str {
    "format = \"veredas-bench\"\nversion = 1\n"
}

/// Why a loaded bench did not start.
fn start_err(spec: BenchSpec) -> String {
    match Bench::start(spec) {
        Ok(_) => panic!("the bench started"),
        Err(e) => e.to_string(),
    }
}

#[test]
fn the_documented_bench_loads_starts_and_every_part_does_its_work() {
    let prose = doc_prose();
    let dir = scratch("example");
    let (path, port) = plant(&dir);
    let spec = load_file(&path).unwrap();
    // The plan, as the prose describes it.
    assert_eq!(spec.seed, Some(7));
    assert_eq!(spec.out_dir, dir.join("logs"));
    assert_eq!(spec.emitters.len(), 1);
    let e = &spec.emitters[0];
    assert_eq!((e.name.as_str(), e.interval_ms, e.max_lines), ("scanner", number_after(&prose, "writes a line every"), number_after(&prose, "rotates every")));
    assert_eq!((e.interval_ms, e.max_lines), (200, 1000));
    assert_eq!(spec.servers.len(), 1);
    let (name, s) = &spec.servers[0];
    assert_eq!((name.as_str(), s.port, s.tags.len(), s.reactions.len()), ("plc", 0, 2, 1));
    let draw = s.tags[0].randomize.as_ref().unwrap();
    assert_eq!((s.tags[0].id.as_str(), draw.low, draw.high, draw.interval_ms), ("speed", 1.0, 2.0, number_after(&prose, "drawn between 1.0 and 2.0 every")));
    assert_eq!((s.tags[1].id.as_str(), s.tags[1].access), ("run", Access::ReadWrite));
    assert_eq!((s.reactions[0].then[0].after_ms, s.reactions[0].then[0].set.as_str()), (number_after(&prose, "setting the speed to 2.0 after"), "2.0"));
    assert_eq!(spec.channels.len(), 2);
    assert_eq!(spec.channels[0].name, "machine");
    assert_eq!(spec.channels[0].role, Role::Listen { bind: "127.0.0.1".into(), port });
    assert_eq!(spec.channels[1].role, Role::Connect { host: "127.0.0.1".into(), port });
    assert_eq!(number_after(&prose, "listens on port"), 5020, "the port the reference names; the test takes a free one");
    assert_eq!(spec.channels[0].controller.as_ref().unwrap().sends(), ["announce", "outcome", "heartbeat", "status"]);
    assert_eq!(spec.channels[1].controller.as_ref().unwrap().sends(), ["answer"]);
    assert_eq!(spec.channels[1].messages.len(), 1);
    let plan = spec.plan();
    assert_eq!(plan.len(), 4, "{plan:?}");
    assert!(plan[0].starts_with("emitter `scanner`: `%Y%m%d_scanner{n}.log` every 200 ms into "), "{}", plan[0]);
    assert_eq!(plan[1], "opc `plc`: opc.tcp://127.0.0.1:0/ with 2 tag(s), 1 reaction(s)");
    assert_eq!(plan[2], format!("channel `machine`: listening on 127.0.0.1:{port}, 0 message(s) of its own, a controller sending [\"announce\", \"outcome\", \"heartbeat\", \"status\"] and receiving [\"answer\", \"command\"]"));
    assert_eq!(plan[3], format!("channel `unit`: dialling 127.0.0.1:{port}, 1 message(s) of its own, a controller sending [\"answer\"] and receiving [\"announce\"]"));

    // Started: every part does its work, seen through the handle.
    let bench = Bench::start(spec).unwrap();
    assert_eq!(bench.status().iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["scanner", "plc", "machine", "unit"]);
    // The emitter writes its lines into a daily file under `logs`.
    assert!(wait(Duration::from_secs(5), || bench.emitters[0].1.status().total_lines >= 3));
    let status = bench.emitters[0].1.status();
    let file = status.current_file.clone().unwrap();
    let today = chrono::Local::now().format("%Y%m%d").to_string();
    assert_eq!(file, format!("{today}_scanner1.log"));
    let text = std::fs::read_to_string(dir.join("logs").join(&file)).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines.len() >= 3, "{text}");
    let line = lines[0];
    assert!(line.contains(" SCAN code=") && line.contains(" weight="), "{line}");
    let code: u64 = line.split("code=").nth(1).unwrap().split(' ').next().unwrap().parse().unwrap();
    let weight: f64 = line.split("weight=").nth(1).unwrap().parse().unwrap();
    assert!((1000..=9999).contains(&code) && (0.1..=30.0).contains(&weight), "{line}");
    assert_eq!(status.last_error, None);
    // The OPC UA server draws its speed and reacts to the run flag.
    let plc = bench.server("plc").unwrap();
    assert!(plc.endpoint_url.starts_with("opc.tcp://127.0.0.1:") && plc.port != 0);
    assert!(wait(Duration::from_secs(5), || plc.status().draws >= 2));
    match plc.get("speed") {
        Some(opcua_types::Variant::Double(v)) => assert!((1.0..=2.0).contains(&v), "{v}"),
        other => panic!("{other:?}"),
    }
    plc.set("run", true);
    assert!(wait(Duration::from_secs(5), || plc.status().reactions_fired >= 1));
    // The machine and the unit converse; the unit's own message reaches the machine.
    assert!(wait(Duration::from_secs(5), || {
        let m = bench.channel("machine").unwrap().status();
        m.peers.len() == 1 && m.controller.as_ref().map(|c| c.stats.answered >= 3).unwrap_or(false)
    }), "{:?}", bench.channel("machine").unwrap().status());
    bench.channel("unit").unwrap().send_now("status", BTreeMap::from([("running".to_string(), "1".to_string()), ("speed".to_string(), "50".to_string())]).into_iter().collect(), None);
    assert!(wait(Duration::from_secs(5), || bench.channel("machine").unwrap().status().received.get("status") == Some(&1)));
    // One line per part.
    let lines = bench.lines();
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(lines[0].starts_with("emitter scanner: ") && lines[0].contains(" line(s), ") && lines[0].contains(&file), "{}", lines[0]);
    assert!(lines[1].starts_with(&format!("opc plc: {} ", plc.endpoint_url)) && lines[1].contains(" draw(s), 1 reaction(s) fired, 0 type mismatch(es)"), "{}", lines[1]);
    assert!(lines[2].starts_with("channel machine: 1 peer(s) (1 peer(s)), ") && lines[2].contains(", controller: ") && lines[2].contains(" announced, "), "{}", lines[2]);
    assert!(lines[3].starts_with("channel unit: connected to 127.0.0.1:") && lines[3].contains(" received, 0 unknown id(s), 0 undecodable, controller: 0 announced"), "{}", lines[3]);
    assert!(bench.channel("nope").is_none() && bench.server("nope").is_none());
    // Stopped: the port is free, the file is whole.
    bench.stop();
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    let text = std::fs::read_to_string(dir.join("logs").join(&file)).unwrap();
    assert!(text.ends_with("\r\n") && text.lines().count() >= 3);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A bench file with every key of every table present, the parts named `e`, `s` and `c`.
fn full_file(port: u16) -> String {
    format!(
        "{}seed = 1\nout_dir = \"out\"\n\n\
         [[emitter]]\nname = \"e\"\ntemplate = \"{{ts}} {{v}}\"\nts_format = \"%H:%M:%S\"\nvalues = [ {{ name = \"v\", low = 1, high = 2, decimals = 1 }} ]\ninterval_ms = 100\nfile = \"e{{n}}.log\"\nmax_lines = 10\nencoding = \"utf-8\"\nseed = 2\n\n\
         [[opc]]\nname = \"s\"\nbind = \"127.0.0.1\"\nport = 0\nuser = {{ name = \"u\", password = \"p\" }}\nmax_nodes_per_read = 10\nmax_sessions = 2\n\
         tags = [ {{ id = \"t\", name = \"T\", type = \"float\", access = \"read-write\", value = \"1\", randomize = {{ low = 1, high = 2, decimals = 1, interval_ms = 100 }} }}, {{ id = \"r\", type = \"float\", value = \"2\", ramp = {{ low = 0, high = 10, step = 2, decimals = 1, interval_ms = 100 }} }} ]\n\
         reactions = [ {{ on_write = \"t\", equals = \"5\", tag = \"t\", then = [ {{ after_ms = 1, set = \"6\" }} ] }} ]\n\n\
         [[modbus]]\nname = \"m\"\nbind = \"127.0.0.2\"\nport = 0\nunit = 1\nmax_connections = 2\ncoils = 4\ndiscrete_inputs = 5\ninput_registers = 6\nholding_registers = 7\n\
         points = [ {{ name = \"p\", table = \"holding-register\", address = 0, type = \"u32\", words = \"low-first\", value = \"1\" }} ]\n\n\
         [[ethernetip]]\nname = \"x\"\nbind = \"127.0.0.3\"\nport = 0\nio_port = 0\n\
         identity = {{ vendor_id = 1, device_type = 2, product_code = 3, revision = \"4.5\", serial = 6, product_name = \"n\" }}\n\
         rpi_min_us = 2000\nrpi_max_us = 3000000\nmax_sessions = 2\nmax_connections = 3\n\
         assemblies = [ {{ instance = 100, size = 4 }}, {{ instance = 150, size = 2 }}, {{ instance = 151, size = 1 }} ]\n\
         connections = [ {{ config = 151, output = 150, input = 100, output_header = \"modeless\", input_header = \"run-idle\" }} ]\n\
         points = [ {{ name = \"q\", assembly = 100, byte = 0, bit = 1, type = \"bool\", value = \"true\" }} ]\n\n\
         [[channel]]\nname = \"c\"\nrole = \"listen\"\nhost = \"127.0.0.1\"\nport = {port}\nchannel = \"channel.toml\"\ncatalog = \"catalog.toml\"\ncontroller = \"machine.toml\"\n\
         messages = [ {{ message = \"status\", every_ms = 100, fields = [ {{ name = \"running\", value = 1 }}, {{ name = \"speed\", value = 1 }} ] }} ]\n",
        header()
    )
}

fn sections(text: &str) -> Vec<(String, Vec<String>)> {
    let mut out: Vec<(String, Vec<String>)> = vec![(String::new(), Vec::new())];
    for line in text.lines() {
        if line.starts_with('[') {
            out.push((line.to_string(), Vec::new()));
        } else if !line.trim().is_empty() {
            out.last_mut().unwrap().1.push(line.to_string());
        }
    }
    out
}

fn join(sections: &[(String, Vec<String>)]) -> String {
    let mut out = String::new();
    for (header, lines) in sections {
        if !header.is_empty() {
            out.push_str(header);
            out.push('\n');
        }
        for l in lines {
            out.push_str(l);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

fn without_line(text: &str, header: &str, key: &str) -> String {
    let mut s = sections(text);
    let section = s.iter_mut().find(|(h, _)| h == header).unwrap_or_else(|| panic!("no section {header}"));
    let before = section.1.len();
    section.1.retain(|l| !l.starts_with(&format!("{key} =")));
    assert_eq!(section.1.len() + 1, before, "no line `{key} =` in {header}");
    join(&s)
}

fn without_sections(text: &str, headers: &[&str]) -> String {
    join(&sections(text).into_iter().filter(|(h, _)| !headers.contains(&h.as_str())).collect::<Vec<_>>())
}

fn reader_keys(err: &str) -> Vec<String> {
    let at = err.find("expected ").unwrap_or_else(|| panic!("{err}"));
    let rest = err[at..].lines().next().unwrap();
    let mut keys: Vec<String> = rest.split('`').skip(1).step_by(2).map(String::from).collect();
    keys.sort();
    keys
}

#[test]
fn every_key_the_reference_lists_is_the_reader_s_and_states_what_the_reader_does() {
    let documented = documented_keys();
    let tables: Vec<&str> = documented.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(
        tables,
        [
            "The top level", "An emitter", "A value", "An OPC UA server", "A user", "A tag", "A draw", "A ramp", "A reaction", "A step", "A Modbus server", "A point",
            "An EtherNet/IP adapter", "An identity", "An assembly", "A connection", "An assembly point", "A channel", "A message"
        ],
        "the reference's nineteen tables"
    );
    let dir = scratch("walk");
    let (_, port) = plant(&dir);
    let full = full_file(port);
    load(&full, &dir).unwrap_or_else(|e| panic!("the full file loads: {e}"));
    let section_of = |table: &str| -> Option<&'static str> {
        Some(match table {
            "The top level" => "",
            "An emitter" => "[[emitter]]",
            "An OPC UA server" => "[[opc]]",
            "A Modbus server" => "[[modbus]]",
            "An EtherNet/IP adapter" => "[[ethernetip]]",
            "A channel" => "[[channel]]",
            _ => return None,
        })
    };
    let inline_full = |table: &str| -> &'static str {
        match table {
            "A value" => "name = \"v\"\nlow = 1\nhigh = 2\ndecimals = 1\n",
            "A user" => "name = \"u\"\npassword = \"p\"\n",
            "A tag" => "id = \"t\"\nname = \"T\"\ntype = \"float\"\naccess = \"read-write\"\nvalue = \"1\"\nrandomize = { low = 1, high = 2 }\nramp = { low = 1, high = 2 }\n",
            "A draw" => "low = 1\nhigh = 2\ndecimals = 1\ninterval_ms = 100\n",
            "A ramp" => "low = 1\nhigh = 2\nstep = 1\ndecimals = 1\ninterval_ms = 100\n",
            "A reaction" => "on_write = \"t\"\nequals = \"5\"\ntag = \"t\"\nthen = [ { set = \"6\" } ]\n",
            "A step" => "after_ms = 1\nset = \"6\"\n",
            "A point" => "name = \"p\"\ntable = \"holding-register\"\naddress = 0\ntype = \"u32\"\nwords = \"low-first\"\nvalue = \"1\"\n",
            "An identity" => "vendor_id = 1\ndevice_type = 2\nproduct_code = 3\nrevision = \"4.5\"\nserial = 6\nproduct_name = \"n\"\n",
            "An assembly" => "instance = 100\nsize = 4\n",
            "A connection" => "config = 151\noutput = 150\ninput = 100\noutput_header = \"modeless\"\ninput_header = \"run-idle\"\n",
            "An assembly point" => "name = \"q\"\nassembly = 100\nbyte = 0\nbit = 1\ntype = \"bool\"\nvalue = \"true\"\n",
            "A message" => "message = \"status\"\nevery_ms = 100\nfields = [ { name = \"running\", value = 1 } ]\n",
            other => panic!("{other}"),
        }
    };
    let drop_line = |text: &str, key: &str| -> String {
        let out: Vec<&str> = text.lines().filter(|l| !l.starts_with(&format!("{key} ="))).collect();
        assert_eq!(out.len() + 1, text.lines().count(), "no line `{key} =` in {text}");
        out.join("\n") + "\n"
    };
    // The reader's own key list of each table, from its refusal of a misspelled key there.
    let misspelled = |table: &str| -> String {
        let zz = "zz = 1\n";
        match table {
            "The top level" => format!("{}{zz}", header()),
            "An emitter" => full.replace("[[emitter]]\n", &format!("[[emitter]]\n{zz}")),
            "An OPC UA server" => full.replace("[[opc]]\n", &format!("[[opc]]\n{zz}")),
            "A Modbus server" => full.replace("[[modbus]]\n", &format!("[[modbus]]\n{zz}")),
            "An EtherNet/IP adapter" => full.replace("[[ethernetip]]\n", &format!("[[ethernetip]]\n{zz}")),
            "An identity" => full.replace("identity = { vendor_id = 1,", "identity = { vendor_id = 1, zz = 1,"),
            "An assembly" => full.replace("{ instance = 100, size = 4 }", "{ instance = 100, zz = 1, size = 4 }"),
            "A connection" => full.replace("{ config = 151,", "{ config = 151, zz = 1,"),
            "An assembly point" => full.replace("{ name = \"q\",", "{ name = \"q\", zz = 1,"),
            "A channel" => full.replace("[[channel]]\n", &format!("[[channel]]\n{zz}")),
            "A value" => full.replace("{ name = \"v\",", "{ name = \"v\", zz = 1,"),
            "A user" => full.replace("{ name = \"u\",", "{ name = \"u\", zz = 1,"),
            "A tag" => full.replace("{ id = \"t\",", "{ id = \"t\", zz = 1,"),
            "A draw" => full.replace("randomize = { low = 1,", "randomize = { low = 1, zz = 1,"),
            "A ramp" => full.replace("ramp = { low = 0,", "ramp = { low = 0, zz = 1,"),
            "A reaction" => full.replace("{ on_write = \"t\",", "{ on_write = \"t\", zz = 1,"),
            "A step" => full.replace("{ after_ms = 1,", "{ after_ms = 1, zz = 1,"),
            "A point" => full.replace("{ name = \"p\",", "{ name = \"p\", zz = 1,"),
            "A message" => full.replace("{ message = \"status\",", "{ message = \"status\", zz = 1,"),
            other => panic!("{other}"),
        }
    };
    for (table, keys) in &documented {
        let text = misspelled(table);
        assert_ne!(text, full, "{table}: no misspelling placed");
        let err = refused(&text, &dir);
        assert!(err.contains("`zz`"), "{table}: {err}");
        let mut names: Vec<String> = keys.iter().map(|(k, _)| k.clone()).collect();
        names.sort();
        assert_eq!(names, reader_keys(&err), "{table}: the documented keys are the reader's");
    }
    // Each key's statement against the reader.
    for (table, keys) in &documented {
        for (key, statement) in keys {
            let required = statement.starts_with("required");
            let default = statement.split(';').next().unwrap().trim().trim_matches('`').to_string();
            if let Some(section) = section_of(table) {
                let text = match (table.as_str(), key.as_str()) {
                    ("The top level", "emitter") => without_sections(&full, &["[[emitter]]"]),
                    ("The top level", "opc") => without_sections(&full, &["[[opc]]"]),
                    ("The top level", "modbus") => without_sections(&full, &["[[modbus]]"]),
                    ("The top level", "ethernetip") => without_sections(&full, &["[[ethernetip]]"]),
                    // Connections and points name the assemblies.
                    ("An EtherNet/IP adapter", "assemblies") => {
                        without_line(&without_line(&without_line(&full, "[[ethernetip]]", "assemblies"), "[[ethernetip]]", "connections"), "[[ethernetip]]", "points")
                    }
                    ("The top level", "channel") => without_sections(&full, &["[[channel]]"]),
                    // A reaction needs its tags.
                    ("An OPC UA server", "tags") => without_line(&without_line(&full, "[[opc]]", "tags"), "[[opc]]", "reactions"),
                    _ => without_line(&full, section, key),
                };
                if required {
                    let err = refused(&text, &dir);
                    let expected = match key.as_str() {
                        "format" => "no `format`: a bench file states `format` and `version`".to_string(),
                        "version" => "no `version`: a bench file states `format` and `version`".to_string(),
                        _ => format!("missing field `{key}`"),
                    };
                    assert!(err.contains(&expected), "{table}.{key} is documented required ({statement}): {err}");
                    continue;
                }
                let spec = load(&text, &dir).unwrap_or_else(|e| panic!("{table}.{key} documented as `{statement}` but refused when left out: {e}"));
                match (table.as_str(), key.as_str()) {
                    ("The top level", "seed") => assert!(default == "none" && spec.seed.is_none()),
                    ("The top level", "out_dir") => assert_eq!((default.as_str(), spec.out_dir.clone()), ("\".\"", dir.join("."))),
                    ("The top level", "emitter") => assert!(default == "none" && spec.emitters.is_empty()),
                    ("The top level", "opc") => assert!(default == "none" && spec.servers.is_empty()),
                    ("The top level", "modbus") => assert!(default == "none" && spec.modbus.is_empty()),
                    ("The top level", "ethernetip") => assert!(default == "none" && spec.ethernetip.is_empty()),
                    ("The top level", "channel") => assert!(default == "none" && spec.channels.is_empty()),
                    ("An emitter", "values") => assert!(default == "none" && spec.emitters[0].values.is_empty()),
                    ("An emitter", "max_lines") => assert_eq!((default.as_str(), spec.emitters[0].max_lines), ("0", 0)),
                    ("An emitter", "encoding") => assert_eq!((default.as_str(), spec.emitters[0].encoding), ("\"utf-8\"", Encoding::Utf8)),
                    ("An emitter", "seed") => assert!(default == "none" && spec.emitters[0].seed.is_none()),
                    ("An OPC UA server", "bind") => assert_eq!((default.as_str(), spec.servers[0].1.bind.as_str()), ("\"127.0.0.1\"", "127.0.0.1")),
                    ("An OPC UA server", "port") => assert_eq!((default.as_str(), spec.servers[0].1.port), ("4855", 4855)),
                    ("An OPC UA server", "user") => assert!(default == "none" && spec.servers[0].1.user.is_none()),
                    ("An OPC UA server", "max_nodes_per_read") => assert!(default == "none" && spec.servers[0].1.max_nodes_per_read.is_none()),
                    ("An OPC UA server", "max_sessions") => assert!(default == "none" && spec.servers[0].1.max_sessions.is_none()),
                    ("An OPC UA server", "tags") => assert!(default == "none" && spec.servers[0].1.tags.is_empty()),
                    ("An OPC UA server", "reactions") => assert!(default == "none" && spec.servers[0].1.reactions.is_empty()),
                    ("A Modbus server", "bind") => assert_eq!((default.as_str(), spec.modbus[0].1.bind.as_str()), ("\"127.0.0.1\"", "127.0.0.1")),
                    ("A Modbus server", "port") => assert_eq!((default.as_str(), spec.modbus[0].1.port), ("502", 502)),
                    ("A Modbus server", "unit") => assert!(default == "none" && spec.modbus[0].1.unit.is_none()),
                    ("A Modbus server", "max_connections") => assert!(default == "none" && spec.modbus[0].1.max_connections.is_none()),
                    ("A Modbus server", "coils") => assert!(default == "none" && spec.modbus[0].1.coils.is_none()),
                    ("A Modbus server", "discrete_inputs") => assert!(default == "none" && spec.modbus[0].1.discrete_inputs.is_none()),
                    ("A Modbus server", "input_registers") => assert!(default == "none" && spec.modbus[0].1.input_registers.is_none()),
                    ("A Modbus server", "holding_registers") => assert!(default == "none" && spec.modbus[0].1.holding_registers.is_none()),
                    ("A Modbus server", "points") => assert!(default == "none" && spec.modbus[0].1.points.is_empty()),
                    ("An EtherNet/IP adapter", "bind") => assert_eq!((default.as_str(), spec.ethernetip[0].1.bind.as_str()), ("\"127.0.0.1\"", "127.0.0.1")),
                    ("An EtherNet/IP adapter", "port") => assert_eq!((default.as_str(), spec.ethernetip[0].1.port), ("44818", 44818)),
                    ("An EtherNet/IP adapter", "io_port") => assert_eq!((default.as_str(), spec.ethernetip[0].1.io_port), ("2222", 2222)),
                    ("An EtherNet/IP adapter", "identity") => assert!(default == "the defaults below" && spec.ethernetip[0].1.identity == Identity::default()),
                    ("An EtherNet/IP adapter", "rpi_min_us") => assert_eq!((default.as_str(), spec.ethernetip[0].1.rpi_min_us), ("1000", 1000)),
                    ("An EtherNet/IP adapter", "rpi_max_us") => assert_eq!((default.as_str(), spec.ethernetip[0].1.rpi_max_us), ("10000000", 10_000_000)),
                    ("An EtherNet/IP adapter", "max_sessions") => assert!(default == "none" && spec.ethernetip[0].1.max_sessions.is_none()),
                    ("An EtherNet/IP adapter", "max_connections") => assert!(default == "none" && spec.ethernetip[0].1.max_connections.is_none()),
                    ("An EtherNet/IP adapter", "assemblies") => assert!(default == "none" && spec.ethernetip[0].1.assemblies.is_empty()),
                    ("An EtherNet/IP adapter", "connections") => assert!(default == "none" && spec.ethernetip[0].1.connections.is_empty()),
                    ("An EtherNet/IP adapter", "points") => assert!(default == "none" && spec.ethernetip[0].1.points.is_empty()),
                    ("A channel", "host") => assert_eq!((default.as_str(), &spec.channels[0].role), ("\"127.0.0.1\"", &Role::Listen { bind: "127.0.0.1".into(), port })),
                    ("A channel", "controller") => assert!(default == "none" && spec.channels[0].controller.is_none()),
                    ("A channel", "messages") => assert!(default == "none" && spec.channels[0].messages.is_empty()),
                    other => panic!("{other:?}: a documented default this test does not know how to check"),
                }
                continue;
            }
            let text = drop_line(inline_full(table), key);
            macro_rules! check {
                ($ty:ty, $ok:expr) => {{
                    let parsed = toml::from_str::<$ty>(&text);
                    if required {
                        let err = parsed.err().map(|e| e.to_string()).unwrap_or_else(|| panic!("{table}.{key} is documented required but loads without it"));
                        assert!(err.contains(&format!("missing field `{key}`")), "{table}.{key}: {err}");
                    } else {
                        let v = parsed.unwrap_or_else(|e| panic!("{table}.{key} documented as `{statement}` but refused when left out: {e}"));
                        let f: &dyn Fn(&$ty) -> bool = &$ok;
                        assert!(f(&v), "{table}.{key}: the default is not `{default}`: {v:?}");
                    }
                }};
            }
            match (table.as_str(), key.as_str()) {
                ("A value", "decimals") => check!(ValueField, |v| default == "0" && v.decimals == 0),
                ("A value", _) => check!(ValueField, |_| false),
                ("A user", _) => check!(Credentials, |_| false),
                ("A tag", "name") => check!(TagConfig, |t| default == "id" && t.name.is_none()),
                ("A tag", "access") => check!(TagConfig, |t| default == "\"read\"" && t.access == Access::Read),
                ("A tag", "value") => check!(TagConfig, |t| default == "the type's zero" && t.value.is_empty()),
                ("A tag", "randomize") => check!(TagConfig, |t| default == "none" && t.randomize.is_none()),
                ("A tag", "ramp") => check!(TagConfig, |t| default == "none" && t.ramp.is_none()),
                ("A tag", _) => check!(TagConfig, |_| false),
                ("A draw", "low") => check!(Randomize, |r| default == "0" && r.low == 0.0),
                ("A draw", "high") => check!(Randomize, |r| default == "0" && r.high == 0.0),
                ("A draw", "decimals") => check!(Randomize, |r| default == "0" && r.decimals == 0),
                ("A draw", "interval_ms") => check!(Randomize, |r| default == "1000" && r.interval_ms == 1000),
                ("A ramp", "step") => check!(Ramp, |r| default == "1" && r.step == 1.0),
                ("A ramp", "decimals") => check!(Ramp, |r| default == "none" && r.decimals.is_none()),
                ("A ramp", "interval_ms") => check!(Ramp, |r| default == "1000" && r.interval_ms == 1000),
                ("A ramp", _) => check!(Ramp, |_| false),
                ("A reaction", "equals") => check!(Reaction, |r| default == "none" && r.equals.is_none()),
                ("A reaction", "tag") => check!(Reaction, |r| default == "on_write" && r.tag.is_none()),
                ("A reaction", _) => check!(Reaction, |_| false),
                ("A step", "after_ms") => check!(Step, |s| default == "0" && s.after_ms == 0),
                ("A step", _) => check!(Step, |_| false),
                ("A point", "type") => check!(PointConfig, |p| default == "bool` in a bit table, `u16` in a register table" && p.ty.is_none()),
                ("A point", "words") => check!(PointConfig, |p| default == "\"high-first\"" && p.words.is_none()),
                ("A point", "value") => check!(PointConfig, |p| default == "the type's zero" && p.value.is_empty()),
                ("A point", _) => check!(PointConfig, |_| false),
                ("An identity", "vendor_id") => check!(Identity, |i| default == "0" && i.vendor_id == 0),
                ("An identity", "device_type") => check!(Identity, |i| default == "12" && i.device_type == 12),
                ("An identity", "product_code") => check!(Identity, |i| default == "0" && i.product_code == 0),
                ("An identity", "revision") => check!(Identity, |i| default == "\"1.1\"" && i.revision == "1.1"),
                ("An identity", "serial") => check!(Identity, |i| default == "0" && i.serial == 0),
                ("An identity", "product_name") => check!(Identity, |i| default == "\"veredas bench adapter\"" && i.product_name == "veredas bench adapter"),
                ("An identity", _) => check!(Identity, |_| false),
                ("An assembly", _) => check!(AssemblyConfig, |_| false),
                ("A connection", "output_header") => check!(ConnectionConfig, |c| default == "\"run-idle\"" && c.output_header.is_none()),
                ("A connection", "input_header") => check!(ConnectionConfig, |c| default == "\"modeless\"" && c.input_header.is_none()),
                ("A connection", _) => check!(ConnectionConfig, |_| false),
                ("An assembly point", "bit") => check!(EnipPoint, |p| default == "none" && p.bit.is_none()),
                ("An assembly point", "type") => check!(EnipPoint, |p| default == "bool` with a bit, `u8` without" && p.ty.is_none()),
                ("An assembly point", "value") => check!(EnipPoint, |p| default == "the type's zero" && p.value.is_empty()),
                ("An assembly point", _) => check!(EnipPoint, |_| false),
                ("A message", "every_ms") => check!(CyclicMessage, |m| default == "0" && m.every_ms == 0),
                ("A message", "fields") => check!(CyclicMessage, |m| default == "none" && m.fields.is_empty()),
                ("A message", _) => check!(CyclicMessage, |_| false),
                other => panic!("{other:?}: a key this test does not know how to check"),
            }
        }
    }
    // The names the reference lists are the reader's.
    let prose = doc_prose();
    assert!(prose.contains("Each key states in parentheses that it is required, or the value it takes when left out. Every part has a name, exact, with no surrounding whitespace, and no two parts share one. A path is relative to the bench file's folder."));
    assert!(prose.contains("`format` (required): always `\"veredas-bench\"`."));
    assert_eq!(FORMAT, "veredas-bench");
    assert!(prose.contains("`version` (required): this reader knows `1`."));
    assert_eq!(VERSION, 1);
    let variants = |err: String| -> Vec<String> {
        let mut v: Vec<String> = err[err.find("expected").unwrap()..].split('`').skip(1).step_by(2).map(String::from).collect();
        v.sort();
        v
    };
    assert!(prose.contains("`type` (required): `int` (an Int32), `float` (a Double), `bool` or `text`."));
    assert_eq!(variants(toml::from_str::<TagConfig>("id = \"t\"\ntype = \"x\"\n").unwrap_err().to_string()), ["bool", "float", "int", "text"]);
    assert_eq!(toml::from_str::<TagConfig>("id = \"t\"\ntype = \"int\"\n").unwrap().ty, TagType::Int);
    assert!(prose.contains("`access` (`\"read\"`): `read`, `write` (a command the controller takes but does not report) or `read-write`."));
    assert_eq!(variants(toml::from_str::<TagConfig>("id = \"t\"\ntype = \"int\"\naccess = \"x\"\n").unwrap_err().to_string()), ["read", "read-write", "write"]);
    assert!(prose.contains("`table` (required): `coil`, `discrete-input`, `input-register` or `holding-register`."));
    let point = |rest: &str| format!("name = \"p\"\naddress = 0\n{rest}");
    assert_eq!(variants(toml::from_str::<PointConfig>(&point("table = \"x\"")).unwrap_err().to_string()), ["coil", "discrete-input", "holding-register", "input-register"]);
    for (word, table) in [("coil", Table::Coil), ("discrete-input", Table::DiscreteInput), ("input-register", Table::InputRegister), ("holding-register", Table::HoldingRegister)] {
        assert_eq!(toml::from_str::<PointConfig>(&point(&format!("table = \"{word}\""))).unwrap().table, table);
    }
    assert!(prose.contains("`type` (`bool` in a bit table, `u16` in a register table): `bool` in a coil or a discrete input; `u16`, `i16`, `u32`, `i32` or `f32` (a single-precision float) in a register table, the last three over two registers."));
    assert_eq!(variants(toml::from_str::<PointConfig>(&point("table = \"coil\"\ntype = \"x\"")).unwrap_err().to_string()), ["bool", "f32", "i16", "i32", "u16", "u32"]);
    assert_eq!(toml::from_str::<PointConfig>(&point("table = \"coil\"\ntype = \"f32\"")).unwrap().ty, Some(PointType::F32));
    assert!(prose.contains("`words` (`\"high-first\"`): which of a two-register point's registers holds the high word, `high-first` (the one at its address) or `low-first`;"));
    assert_eq!(variants(toml::from_str::<PointConfig>(&point("table = \"coil\"\nwords = \"x\"")).unwrap_err().to_string()), ["high-first", "low-first"]);
    assert_eq!(toml::from_str::<PointConfig>(&point("table = \"coil\"\nwords = \"low-first\"")).unwrap().words, Some(WordOrder::LowFirst));
    // Every key of an EtherNet/IP adapter reaches its configuration, each with a value of its own.
    let x = load(&full, &dir).unwrap();
    assert_eq!(x.ethernetip[0].0, "x");
    assert_eq!(
        x.ethernetip[0].1,
        EnipConfig {
            bind: "127.0.0.3".into(),
            port: 0,
            io_port: 0,
            identity: Identity { vendor_id: 1, device_type: 2, product_code: 3, revision: "4.5".into(), serial: 6, product_name: "n".into() },
            rpi_min_us: 2000,
            rpi_max_us: 3_000_000,
            max_sessions: Some(2),
            max_connections: Some(3),
            assemblies: vec![AssemblyConfig { instance: 100, size: 4 }, AssemblyConfig { instance: 150, size: 2 }, AssemblyConfig { instance: 151, size: 1 }],
            connections: vec![ConnectionConfig { config: 151, output: 150, input: 100, output_header: Some(DataHeader::Modeless), input_header: Some(DataHeader::RunIdle) }],
            points: vec![EnipPoint { name: "q".into(), assembly: 100, byte: 0, bit: Some(1), ty: Some(EnipType::Bool), value: "true".into() }],
        }
    );
    let conn = |rest: &str| format!("config = 1\noutput = 2\ninput = 3\n{rest}");
    assert!(prose.contains("`output_header` (`\"run-idle\"`): what the consumed data carries before its bytes: `run-idle` (a 32-bit run/idle header) or `modeless`."));
    assert_eq!(variants(toml::from_str::<ConnectionConfig>(&conn("output_header = \"x\"")).unwrap_err().to_string()), ["modeless", "run-idle"]);
    assert!(prose.contains("`type` (`bool` with a bit, `u8` without): `bool`, `u8`, `i8`, `u16`, `i16`, `u32`, `i32` or `f32`."));
    assert_eq!(
        variants(toml::from_str::<EnipPoint>("name = \"q\"\nassembly = 1\nbyte = 0\ntype = \"x\"").unwrap_err().to_string()),
        ["bool", "f32", "i16", "i32", "i8", "u16", "u32", "u8"]
    );
    // Every key of a Modbus server reaches its configuration, each with a value of its own.
    let m = load(&full, &dir).unwrap();
    assert_eq!(m.modbus[0].0, "m");
    assert_eq!(
        m.modbus[0].1,
        ModbusConfig {
            bind: "127.0.0.2".into(),
            port: 0,
            unit: Some(1),
            max_connections: Some(2),
            coils: Some(4),
            discrete_inputs: Some(5),
            input_registers: Some(6),
            holding_registers: Some(7),
            points: vec![PointConfig {
                name: "p".into(),
                table: Table::HoldingRegister,
                address: 0,
                ty: Some(PointType::U32),
                words: Some(WordOrder::LowFirst),
                value: "1".into(),
            }],
        }
    );
    assert!(prose.contains("`encoding` (`\"utf-8\"`): `\"utf-8\"` or `\"utf-16le\"`, the latter with a byte-order mark."));
    assert_eq!(variants(refused(&full.replace("encoding = \"utf-8\"", "encoding = \"x\""), &dir)), ["utf-16le", "utf-8"]);
    assert!(prose.contains("`role` (required): `listen` (wait for the peer to connect) or `connect` (dial it)."));
    assert_eq!(variants(refused(&full.replace("role = \"listen\"", "role = \"x\""), &dir)), ["connect", "listen"]);
    let dialling = load(&full.replace("role = \"listen\"", "role = \"connect\""), &dir).unwrap();
    assert_eq!(dialling.channels[0].role, Role::Connect { host: "127.0.0.1".into(), port });
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_header_is_refused_four_ways_and_a_misspelled_key_in_any_table() {
    let dir = scratch("header");
    assert!(doc_prose().contains("A file of another format or an unknown version is refused, never guessed."));
    assert_eq!(refused("seed = 1\n", &dir), "no `format`: a bench file states `format` and `version`");
    assert_eq!(refused("format = \"veredas-bench\"\n", &dir), "no `version`: a bench file states `format` and `version`");
    assert_eq!(refused("format = \"veredas-catalog\"\nversion = 1\n", &dir), "`format` is `veredas-catalog`, not `veredas-bench`");
    assert_eq!(refused("format = \"veredas-bench\"\nversion = 2\n", &dir), "version 2 of `veredas-bench` is not known here (this reader knows 1)");
    assert!(refused("format = \"veredas-bench\"\nversion = 2\nnew_key = 1\n", &dir).starts_with("version 2"), "the header is judged before the body");
    assert!(refused("not toml [", &dir).starts_with("not a bench file: "));
    assert!(matches!(load("format = \"x\"\nversion = 1\n", &dir), Err(BenchError::WrongFormat(f)) if f == "x"));
    assert!(matches!(load("format = \"veredas-bench\"\nversion = 0\n", &dir), Err(BenchError::UnknownVersion(0))));
    // The emptiest bench: nothing to run, and it runs.
    let nothing = load(header(), &dir).unwrap();
    assert!(nothing.plan().is_empty());
    let bench = Bench::start(nothing).unwrap();
    assert!(bench.status().is_empty() && bench.lines().is_empty());
    bench.stop();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn every_invalid_bench_is_refused_with_its_place_named_before_anything_starts() {
    let dir = scratch("invalid");
    let (_, port) = plant(&dir);
    let h = header();
    let emitter = |body: &str| format!("{h}[[emitter]]\nname = \"e\"\ntemplate = \"{{ts}}\"\nts_format = \"%H\"\ninterval_ms = 1\nfile = \"e.log\"\n{body}");
    assert_eq!(refused(&format!("{h}[[emitter]]\nname = \" \"\ntemplate = \"x\"\nts_format = \"%H\"\ninterval_ms = 1\nfile = \"e.log\"\n"), &dir), "emitter[0]: the emitter has no name");
    assert_eq!(refused(&format!("{h}[[emitter]]\nname = \"e \"\ntemplate = \"x\"\nts_format = \"%H\"\ninterval_ms = 1\nfile = \"e.log\"\n"), &dir), "emitter[0]: the name `e ` has surrounding whitespace");
    assert_eq!(refused(&emitter("[[opc]]\nname = \"e\"\n"), &dir), "opc[0]: `e` is already the name of the emitter `e`");
    assert_eq!(refused(&emitter("").replace("file = \"e.log\"", "file = \"\""), &dir), "emitter[0] (`e`).file: the file pattern is empty");
    assert_eq!(refused(&emitter("").replace("file = \"e.log\"", "file = \"%H:%M.log\""), &dir), "emitter[0] (`e`).file: a file name cannot contain `:`");
    assert_eq!(refused(&emitter("").replace("ts_format = \"%H\"", "ts_format = \"%\""), &dir), "emitter[0] (`e`).ts_format: `%` is not a datetime format");
    assert_eq!(refused(&emitter("values = [ { name = \" \", low = 1, high = 2 } ]\n"), &dir), "emitter[0] (`e`).values[0]: the value has no name");
    assert_eq!(refused(&emitter("values = [ { name = \"v\", low = 2, high = 1 } ]\n"), &dir), "emitter[0] (`e`).values[0] (`v`): the range 2..=1 is empty or not finite");
    assert_eq!(refused(&emitter("values = [ { name = \"v\", low = 1, high = inf } ]\n"), &dir), "emitter[0] (`e`).values[0] (`v`): the range 1..=inf is empty or not finite");
    assert_eq!(refused(&emitter("values = [ { name = \"v\", low = 1, high = 2, decimals = 16 } ]\n"), &dir), "emitter[0] (`e`).values[0] (`v`): decimals is 16, over 15");
    // The server's own checks, with the part named before them.
    assert_eq!(
        refused(&format!("{h}[[opc]]\nname = \"s\"\ntags = [ {{ id = \"z\", type = \"int\", randomize = {{ interval_ms = 0 }} }} ]\n"), &dir),
        "opc[0] (`s`): opc.tags[0] (`z`).randomize.interval_ms is 0"
    );
    assert_eq!(refused(&format!("{h}[[opc]]\nname = \"s\"\nbind = \" \"\n"), &dir), "opc[0] (`s`): opc.bind is empty");
    assert_eq!(refused(&emitter("[[modbus]]\nname = \"e\"\n"), &dir), "modbus[0]: `e` is already the name of the emitter `e`");
    assert_eq!(refused(&emitter("[[ethernetip]]\nname = \"e\"\n"), &dir), "ethernetip[0]: `e` is already the name of the emitter `e`");
    assert_eq!(
        refused(&format!("{h}[[ethernetip]]\nname = \"x\"\nassemblies = [ {{ instance = 5, size = 1 }}, {{ instance = 5, size = 2 }} ]\n"), &dir),
        "ethernetip[0] (`x`): ethernetip.assemblies[1]: instance 5 is declared twice"
    );
    assert_eq!(
        refused(&format!("{h}[[ethernetip]]\nname = \"x\"\nassemblies = [ {{ instance = 5, size = 1 }} ]\npoints = [ {{ name = \"p\", assembly = 5, byte = 1 }} ]\n"), &dir),
        "ethernetip[0] (`x`): ethernetip.points[0] (`p`): a u8 at byte 1 runs past assembly 5's 1 byte(s)"
    );
    assert_eq!(
        refused(&format!("{h}[[modbus]]\nname = \"m\"\npoints = [ {{ name = \"a\", table = \"coil\", address = 0, type = \"u16\" }} ]\n"), &dir),
        "modbus[0] (`m`): modbus.points[0] (`a`): a coil holds a bool, not u16"
    );
    assert_eq!(
        refused(&format!("{h}[[modbus]]\nname = \"m\"\nholding_registers = 1\npoints = [ {{ name = \"a\", table = \"holding-register\", address = 1 }} ]\n"), &dir),
        "modbus[0] (`m`): modbus.points: point `a` (u16 at 1) is past the end of the holding-register table (holding_registers = 1)"
    );
    // The channel's files and rules.
    let channel = |body: &str| format!("{h}[[channel]]\nname = \"c\"\nrole = \"listen\"\nport = {port}\nchannel = \"channel.toml\"\ncatalog = \"catalog.toml\"\n{body}");
    assert_eq!(refused(&channel("host = \" \"\n"), &dir), "channel[0] (`c`).host is empty");
    let err = refused(&channel("").replace("channel = \"channel.toml\"", "channel = \"nope.toml\""), &dir);
    assert!(err.starts_with(&format!("channel[0] (`c`).channel: read {}: ", dir.join("nope.toml").display())), "{err}");
    std::fs::write(dir.join("bad-channel.toml"), b"format = \"veredas-catalog\"\nversion = 1\n").unwrap();
    assert_eq!(
        refused(&channel("").replace("channel = \"channel.toml\"", "channel = \"bad-channel.toml\""), &dir),
        format!("channel[0] (`c`).channel ({}): `format` is `veredas-catalog`, not `veredas-channel`", dir.join("bad-channel.toml").display())
    );
    std::fs::write(dir.join("bad-catalog.toml"), b"format = \"veredas-catalog\"\nversion = 1\nid_field = \"nope\"\n").unwrap();
    assert_eq!(
        refused(&channel("").replace("catalog = \"catalog.toml\"", "catalog = \"bad-catalog.toml\""), &dir),
        "channel[0] (`c`): the catalog's id_field `nope` is no field of the framing's segment header"
    );
    std::fs::write(dir.join("bad-rules.toml"), b"format = \"veredas-controller\"\nversion = 1\n[items]\nmessage = \"nope\"\ninterval_ms = 1\nkey = \"k\"\nfields = []\n").unwrap();
    assert_eq!(
        refused(&channel("controller = \"bad-rules.toml\"\n"), &dir),
        format!("channel[0] (`c`).controller ({}): items.message: the catalog has no message `nope`", dir.join("bad-rules.toml").display())
    );
    assert_eq!(refused(&channel("messages = [ { message = \"nope\" } ]\n"), &dir), "channel[0] (`c`).messages[0]: the catalog has no message `nope`");
    assert_eq!(
        refused(&channel("messages = [ { message = \"status\", fields = [ { name = \"running\", value = 1 } ] } ]\n"), &dir),
        "channel[0] (`c`).messages[0] (`status`): channel[0] (`c`).messages[0]: the message needs `speed`, which no field writes"
    );
    assert_eq!(
        refused(&channel("messages = [ { message = \"status\", fields = [ { name = \"running\", from = \"x\" }, { name = \"speed\", value = 1 } ] } ]\n"), &dir),
        "channel[0] (`c`).messages[0] (`status`): channel[0] (`c`).messages[0].fields[0] (`running`): `from` needs a message or an item to read from"
    );
    // The closing paragraph says so.
    let prose = doc_prose();
    assert!(prose.contains("A misspelled key in any table, a part without a name or with a name another part has, an emitter's file name or timestamp format that is bad, a value's range that is empty or not finite, anything the OPC UA or the Modbus server's own checks refuse (a point over another or past its table's end, a type its table cannot hold, a value that does not parse) or the EtherNet/IP adapter's (an assembly declared twice, a connection naming an assembly there is not, a point past its assembly's bytes or over another), a file a channel points at that cannot be read or does not load, an `id_field` that does not ride in the framing, a controller file that does not load against the catalog, a message the catalog lacks or whose fields leave a name unwritten or use a source a plain channel lacks, is refused when the bench file loads, with the place named, before anything binds, opens or spawns."));
    // A part that cannot start: a server on a taken port, named; the emitter before it stopped.
    assert!(prose.contains("A part that cannot start (a port taken, a folder that cannot be created) stops the parts started before it and is named."));
    let squatter = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let taken = squatter.local_addr().unwrap().port();
    let spec = load(&format!("{}[[opc]]\nname = \"s\"\nport = {taken}\n", emitter("")), &dir).unwrap();
    let err = start_err(spec);
    assert!(err.starts_with(&format!("opc `s`: bind 127.0.0.1:{taken}: ")), "{err}");
    // The emitter was stopped and joined before the error came back: whatever its file holds then
    // (its first line, or nothing yet when the bind failed before its thread's first tick, as a
    // loaded runner showed) is all it ever holds; a running emitter adds a line every 100 ms.
    let lines = |p: &Path| std::fs::read_to_string(p).map(|t| t.lines().count()).unwrap_or(0);
    let written = lines(&dir.join("e.log"));
    std::thread::sleep(Duration::from_millis(250));
    assert_eq!(lines(&dir.join("e.log")), written, "the emitter was stopped and joined before the error came back");
    // Joined, not merely told to stop: a server started before the failing one has released
    // its port the moment the error is back.
    let q = free_port();
    let spec = load(&format!("{h}[[opc]]\nname = \"a\"\nport = {q}\n\n[[opc]]\nname = \"b\"\nport = {taken}\n"), &dir).unwrap();
    assert!(start_err(spec).starts_with("opc `b`: bind "));
    assert!(std::net::TcpListener::bind(("127.0.0.1", q)).is_ok(), "the first server's port is free at once");
    // A Modbus server on a taken port, named, the OPC UA server before it stopped and its port free.
    let q = free_port();
    let spec = load(&format!("{h}[[opc]]\nname = \"a\"\nport = {q}\n\n[[modbus]]\nname = \"b\"\nport = {taken}\n"), &dir).unwrap();
    let err = start_err(spec);
    assert!(err.starts_with(&format!("modbus `b`: bind 127.0.0.1:{taken}: ")), "{err}");
    assert!(std::net::TcpListener::bind(("127.0.0.1", q)).is_ok(), "the server before it stopped");
    // OPC UA servers start before Modbus servers, whatever the file's order: with both on taken
    // ports, the OPC UA one is the one named.
    let squatter2 = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let taken2 = squatter2.local_addr().unwrap().port();
    let spec = load(&format!("{h}[[modbus]]\nname = \"m\"\nport = {taken2}\n\n[[opc]]\nname = \"o\"\nport = {taken}\n"), &dir).unwrap();
    let err = start_err(spec);
    assert!(err.starts_with(&format!("opc `o`: bind 127.0.0.1:{taken}: ")), "{err}");
    // Modbus servers before EtherNet/IP adapters, whatever the file's order.
    let spec = load(&format!("{h}[[ethernetip]]\nname = \"x\"\nport = {taken}\nio_port = 0\n\n[[modbus]]\nname = \"m\"\nport = {taken2}\n"), &dir).unwrap();
    let err = start_err(spec);
    assert!(err.starts_with(&format!("modbus `m`: bind 127.0.0.1:{taken2}: ")), "{err}");
    let spec = load(&format!("{h}[[ethernetip]]\nname = \"x\"\nport = {taken}\nio_port = 0\n"), &dir).unwrap();
    let err = start_err(spec);
    assert!(err.starts_with(&format!("ethernetip `x`: bind 127.0.0.1:{taken} (tcp): ")), "{err}");
    // Servers start before channels: a channel that would dial the test's own socket never
    // dials when a server before it fails.
    let ear = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    ear.set_nonblocking(true).unwrap();
    let spec = load(
        &format!("{h}[[opc]]\nname = \"s\"\nport = {taken}\n\n[[channel]]\nname = \"c\"\nrole = \"connect\"\nport = {}\nchannel = \"channel.toml\"\ncatalog = \"catalog.toml\"\n", ear.local_addr().unwrap().port()),
        &dir,
    )
    .unwrap();
    assert!(start_err(spec).starts_with("opc `s`: bind "));
    std::thread::sleep(Duration::from_millis(300));
    assert!(matches!(ear.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock), "nobody dialled: the channel after the server was never started");
    // A Modbus server starts before the channels too.
    let spec = load(
        &format!("{h}[[modbus]]\nname = \"m\"\nport = {taken}\n\n[[channel]]\nname = \"c\"\nrole = \"connect\"\nport = {}\nchannel = \"channel.toml\"\ncatalog = \"catalog.toml\"\n", ear.local_addr().unwrap().port()),
        &dir,
    )
    .unwrap();
    assert!(start_err(spec).starts_with("modbus `m`: bind "));
    std::thread::sleep(Duration::from_millis(300));
    assert!(matches!(ear.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock), "nobody dialled: the channel after the Modbus server was never started");
    drop(squatter2);
    let blocker = dir.join("blocker");
    std::fs::write(&blocker, b"a file where the folder should be").unwrap();
    let spec = load(&emitter("").replace(h, &format!("{h}out_dir = \"blocker\"\n")), &dir).unwrap();
    let err = start_err(spec);
    assert!(err.starts_with(&format!("out_dir: read {}: ", blocker.display())), "{err}");
    drop(squatter);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_parts_start_and_show_emitters_first_then_servers_then_channels_each_in_the_file_s_order() {
    assert!(doc_prose().contains("emitters first, then servers (OPC UA, then Modbus, then EtherNet/IP), then channels, each kind in the file's order (TOML groups a kind's tables under one key, so a file keeps no order across kinds)"));
    let dir = scratch("order");
    let (_, port) = plant(&dir);
    let text = format!(
        "{}[[modbus]]\nname = \"sixth\"\nport = 0\n\n[[opc]]\nname = \"first\"\nport = 0\n\n[[channel]]\nname = \"second\"\nrole = \"listen\"\nport = {port}\nchannel = \"channel.toml\"\ncatalog = \"catalog.toml\"\n\n[[emitter]]\nname = \"third\"\ntemplate = \"{{ts}}\"\nts_format = \"%H\"\ninterval_ms = 100\nfile = \"t.log\"\n\n[[emitter]]\nname = \"fourth\"\ntemplate = \"{{ts}}\"\nts_format = \"%H\"\ninterval_ms = 100\nfile = \"f.log\"\n\n[[opc]]\nname = \"fifth\"\nport = 0\n\n[[modbus]]\nname = \"seventh\"\nport = 0\n\n[[ethernetip]]\nname = \"eighth\"\nport = 0\nio_port = 0\n",
        header()
    );
    let spec = load(&text, &dir).unwrap();
    let order = |lines: &[String]| lines.iter().map(|l| l.split('`').nth(1).unwrap_or("").to_string()).collect::<Vec<_>>();
    assert_eq!(order(&spec.plan()), ["third", "fourth", "first", "fifth", "sixth", "seventh", "eighth", "second"]);
    let bench = Bench::start(spec).unwrap();
    assert_eq!(bench.status().iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(), ["third", "fourth", "first", "fifth", "sixth", "seventh", "eighth", "second"]);
    // A Modbus server by name: the one named, none for a name no part has.
    assert!(bench.modbus("nope").is_none() && bench.modbus("first").is_none());
    let line_of = |name: &str| bench.lines().into_iter().find(|l| l.starts_with(&format!("modbus {name}: "))).unwrap();
    for name in ["sixth", "seventh"] {
        let addr = bench.modbus(name).unwrap().addr;
        assert!(line_of(name).starts_with(&format!("modbus {name}: {addr} ")), "{}", line_of(name));
    }
    assert_ne!(bench.modbus("sixth").unwrap().addr, bench.modbus("seventh").unwrap().addr);
    // An EtherNet/IP adapter by name, its line carrying the addresses it got.
    assert!(bench.ethernetip("nope").is_none() && bench.ethernetip("seventh").is_none());
    let eighth = bench.ethernetip("eighth").unwrap();
    let line = bench.lines().into_iter().find(|l| l.starts_with("ethernetip eighth: ")).unwrap();
    assert!(line.starts_with(&format!("ethernetip eighth: {} (class 1 on {}) 0 session(s), 0 connection(s) open", eighth.addr, eighth.io_addr)), "{line}");
    assert_eq!(bench.lines().iter().map(|l| l.split(' ').nth(1).unwrap().trim_end_matches(':').to_string()).collect::<Vec<_>>(), ["third", "fourth", "first", "fifth", "sixth", "seventh", "eighth", "second"]);
    bench.stop();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_bench_s_seed_feeds_every_part_without_a_seed_of_its_own() {
    assert!(doc_prose().contains("each emitter without a seed of its own draws from this seed plus its index, each channel and its controller from this seed; the clock when absent."));
    // Two emitters without seeds: the same lines run for run, different lines from each other.
    let lines_of = |seed: u64, tag: &str| -> (Vec<String>, Vec<String>) {
        let dir = scratch(tag);
        let text = format!(
            "{}seed = {seed}\n[[emitter]]\nname = \"a\"\ntemplate = \"{{v}}\"\nts_format = \"%H\"\nvalues = [ {{ name = \"v\", low = 0, high = 1000000 }} ]\ninterval_ms = 1\nfile = \"a.log\"\n\n[[emitter]]\nname = \"b\"\ntemplate = \"{{v}}\"\nts_format = \"%H\"\nvalues = [ {{ name = \"v\", low = 0, high = 1000000 }} ]\ninterval_ms = 1\nfile = \"b.log\"\n",
            header()
        );
        let bench = Bench::start(load(&text, &dir).unwrap()).unwrap();
        assert!(wait(Duration::from_secs(5), || bench.emitters.iter().all(|(_, e)| e.status().total_lines >= 5)));
        bench.stop();
        let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap().lines().take(5).map(String::from).collect::<Vec<_>>();
        let out = (read("a.log"), read("b.log"));
        std::fs::remove_dir_all(&dir).unwrap();
        out
    };
    let (a1, b1) = lines_of(7, "seed1");
    let (a2, b2) = lines_of(7, "seed2");
    assert_eq!(a1, a2, "the same bench seed, the same lines");
    assert_eq!(b1, b2);
    assert_ne!(a1, b1, "two emitters, two draws");
    let (a3, _) = lines_of(8, "seed3");
    assert_ne!(a1, a3, "another bench seed, other lines");
    // The machine's controller draws from the bench's seed too: the worked example run twice
    // announces item 1 with the same weight, and with another bench seed another weight.
    let first_weights = |seed: u64, tag: &str| -> Vec<String> {
        let dir = scratch(tag);
        let (path, _) = plant(&dir);
        let text = std::fs::read_to_string(&path).unwrap().replace("seed = 7\n", &format!("seed = {seed}\n"));
        std::fs::write(&path, text).unwrap();
        let bench = Bench::start(load_file(&path).unwrap()).unwrap();
        let machine = bench.channel("machine").unwrap();
        let seen = |c: &veredas_maquinar::controller::ControllerStatus, key: &str| c.awaiting.iter().chain(c.history.iter()).any(|i| i.key == key);
        assert!(wait(Duration::from_secs(5), || machine.status().controller.is_some_and(|c| seen(&c, "1") && seen(&c, "2"))), "items 1 and 2 announced and no longer travelling: {:?}", machine.status().controller);
        let c = machine.status().controller.unwrap();
        let items: Vec<_> = c.awaiting.iter().chain(c.history.iter()).cloned().collect();
        let weight = |key: &str| items.iter().find(|i| i.key == key).map(|i| i.fields["weight"].clone()).unwrap_or_default();
        let out = vec![weight("1"), weight("2")];
        bench.stop();
        std::fs::remove_dir_all(&dir).unwrap();
        out
    };
    let w1 = first_weights(7, "seedm1");
    assert_eq!(w1, first_weights(7, "seedm2"), "the rules file says seed 7 too, but the bench's seed is the one used");
    assert_ne!(w1, first_weights(8, "seedm3"));
    // A channel's own messages draw from the bench's seed too: a status line with a drawn
    // speed every 300 ms (a margin for the dial: a draw before the peer is there would shift
    // what it sees), read by a peer the test holds.
    let speeds = |seed: u64, tag: &str| -> Vec<String> {
        let dir = scratch(tag);
        let (_, port) = plant(&dir);
        let ear = veredas_maquinar::channel::start(veredas_maquinar::channel::ChannelConfig {
            name: "ear".into(),
            role: Role::Listen { bind: "127.0.0.1".into(), port },
            spec: veredas_core::telegram::schema::load(&blocks("channel-schema.md")[0]).unwrap(),
            catalog: veredas_core::telegram::catalog::load(&blocks("controller-schema.md")[0]).unwrap(),
            messages: Vec::new(),
            controller: None,
            seed: None,
        })
        .unwrap();
        let text = format!(
            "{}seed = {seed}\n[[channel]]\nname = \"mouth\"\nrole = \"connect\"\nport = {port}\nchannel = \"channel.toml\"\ncatalog = \"catalog.toml\"\nmessages = [ {{ message = \"status\", every_ms = 300, fields = [ {{ name = \"running\", value = 1 }}, {{ name = \"speed\", range = [0, 1000000] }} ] }} ]\n",
            header()
        );
        let bench = Bench::start(load(&text, &dir).unwrap()).unwrap();
        assert!(wait(Duration::from_secs(5), || ear.status().recent.iter().filter(|l| l.message == "status").count() >= 4), "{:?}", ear.status());
        let mut lines: Vec<String> = ear.status().recent.iter().filter(|l| l.message == "status").map(|l| l.fields["speed"].clone()).collect();
        lines.reverse();
        lines.truncate(4);
        bench.stop();
        ear.stop();
        std::fs::remove_dir_all(&dir).unwrap();
        lines
    };
    let s1 = speeds(7, "seedc1");
    assert_eq!(s1.len(), 4);
    assert_eq!(s1, speeds(7, "seedc2"), "the channel's draws under the bench's seed");
    assert_ne!(s1, speeds(8, "seedc3"));
}

/// The port in a status line after `lead` (`127.0.0.1:<port>` or `opc.tcp://127.0.0.1:<port>/`).
fn port_after(line: &str, lead: &str) -> u16 {
    let rest = line.strip_prefix(lead).unwrap_or_else(|| panic!("`{line}` does not start with `{lead}`"));
    rest.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or_else(|_| panic!("no port in `{line}`"))
}

#[test]
fn the_executable_prints_every_part_s_line_at_the_start_with_the_address_it_got() {
    assert!(doc_prose().contains("`veredas-maquinar <bench.toml>` runs it from the command line"));
    let exe = env!("CARGO_BIN_EXE_veredas-maquinar");
    let dir = scratch("start-lines");
    let path = dir.join("bench.toml");
    std::fs::write(
        &path,
        format!("{}[[modbus]]\nname = \"drive\"\nport = 0\nunit = 1\npoints = [ {{ name = \"sp\", table = \"holding-register\", address = 0, value = \"7\" }} ]\n\n[[opc]]\nname = \"plc\"\nport = 0\n", header()),
    )
    .unwrap();
    // `stop` is in its stdin before it runs: whatever it prints comes before its first second, so
    // the status lines are the start's own.
    let mut child = Command::new(exe).arg(&path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    child.stdin.take().unwrap().write_all(b"stop\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 6, "{stdout}");
    assert_eq!(lines[0], format!("{}: 2 part(s) running; close stdin or end the process to stop", path.display()));
    assert_eq!(lines[1], "  opc `plc`: opc.tcp://127.0.0.1:0/ with 0 tag(s), 0 reaction(s)");
    assert_eq!(lines[2], "  modbus `drive`: 127.0.0.1:0 answering unit 1 with 1 point(s)");
    assert_ne!(port_after(lines[3], "opc plc: opc.tcp://127.0.0.1:"), 0, "{stdout}");
    assert_ne!(port_after(lines[4], "modbus drive: 127.0.0.1:"), 0, "{stdout}");
    assert!(lines[4].ends_with(" 0 connection(s), 0 request(s) answered, 0 exception(s), 0 unanswered"), "{stdout}");
    assert_eq!(lines[5], "stopped");
    // The address in the start's line is the one the server listens on: a client reads through it.
    let mut child = Command::new(exe).arg(&path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    let port = loop {
        line.clear();
        assert_ne!(reader.read_line(&mut line).unwrap(), 0, "the executable ended before its Modbus line");
        if line.starts_with("modbus drive: ") {
            break port_after(line.trim_end(), "modbus drive: 127.0.0.1:");
        }
    };
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    c.write_all(&[0, 1, 0, 0, 0, 6, 1, 0x03, 0, 0, 0, 1]).unwrap();
    let mut answer = [0u8; 11];
    std::io::Read::read_exact(&mut c, &mut answer).unwrap();
    assert_eq!(answer, [0, 1, 0, 0, 0, 5, 1, 0x03, 2, 0, 7], "the point's value, read through the printed port");
    child.stdin.take().unwrap().write_all(b"stop\n").unwrap();
    assert!(child.wait().unwrap().success());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_executable_says_its_version_and_stops_on_a_termination_signal() {
    let exe = env!("CARGO_BIN_EXE_veredas-maquinar");
    let out = Command::new(exe).arg("--version").output().unwrap();
    assert_eq!((out.status.code(), String::from_utf8_lossy(&out.stdout).trim().to_string()), (Some(0), format!("veredas-maquinar {}", env!("CARGO_PKG_VERSION"))));
    assert!(out.stderr.is_empty());
    // With a file named too, the version alone: nothing read, nothing started.
    let out = Command::new(exe).arg("missing.toml").arg("--version").output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    // SIGTERM (a service manager's stop) and SIGINT (a terminal's) end a run as closing stdin does:
    // every part stopped, `stopped` printed, exit 0, the ports free.
    #[cfg(unix)]
    for signal in ["-TERM", "-INT"] {
        let dir = scratch("signal");
        let path = dir.join("bench.toml");
        std::fs::write(&path, format!("{}[[modbus]]\nname = \"drive\"\nport = 0\n", header())).unwrap();
        let mut child = Command::new(exe).arg(&path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
        let mut reader = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        let port = loop {
            line.clear();
            assert_ne!(reader.read_line(&mut line).unwrap(), 0, "ended before its Modbus line");
            if line.starts_with("modbus drive: ") {
                break port_after(line.trim_end(), "modbus drive: 127.0.0.1:");
            }
        };
        // Stdin stays open: only the signal can stop it.
        let status = Command::new("kill").arg(signal).arg(child.id().to_string()).status().unwrap();
        assert!(status.success());
        let mut rest = String::new();
        std::io::Read::read_to_string(&mut reader, &mut rest).unwrap();
        let exit = child.wait().unwrap();
        assert_eq!(exit.code(), Some(0), "{signal}: {exit:?}");
        assert_eq!(rest.lines().last(), Some("stopped"), "{signal}: {rest}");
        assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(), "{signal}: the port is free");
        drop(child.stdin.take());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn the_executable_checks_and_runs_a_bench_file() {
    let exe = env!("CARGO_BIN_EXE_veredas-maquinar");
    let dir = scratch("exe");
    let (path, port) = plant(&dir);
    // --check: the plan, nothing started.
    let out = Command::new(exe).arg(&path).arg("--check").output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(out.status.success(), "{stdout}{}", String::from_utf8_lossy(&out.stderr));
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], format!("{}: 4 part(s)", path.display()));
    assert!(lines[1].starts_with("  emitter `scanner`: ") && lines[2].starts_with("  opc `plc`: ") && lines[3].starts_with("  channel `machine`: listening on ") && lines[4].starts_with("  channel `unit`: dialling "), "{stdout}");
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(), "nothing bound");
    assert!(!dir.join("logs").exists(), "nothing written");
    // A refused file: exit 2, the place on stderr.
    std::fs::write(dir.join("bad.toml"), b"format = \"veredas-bench\"\nversion = 1\n[[emitter]]\nname = \" \"\ntemplate = \"x\"\nts_format = \"%H\"\ninterval_ms = 1\nfile = \"e.log\"\n").unwrap();
    let out = Command::new(exe).arg(dir.join("bad.toml")).arg("--check").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&out.stderr).trim(), format!("{}: emitter[0]: the emitter has no name", dir.join("bad.toml").display()));
    let out = Command::new(exe).arg(dir.join("missing.toml")).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).starts_with(&format!("{}: the bench file: read ", dir.join("missing.toml").display())));
    // No file, or an unknown option: exit 1 with the usage.
    let out = Command::new(exe).output().unwrap();
    assert_eq!((out.status.code(), String::from_utf8_lossy(&out.stderr).trim()), (Some(1), "usage: veredas-maquinar <bench.toml> [--check] | --version"));
    let out = Command::new(exe).arg(&path).arg("--nope").output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    // A run: every part's status line once a second, until stdin says stop.
    let mut child = Command::new(exe).arg(&path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let mut seen: Vec<String> = Vec::new();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(15) {
        if let Ok(line) = rx.recv_timeout(Duration::from_millis(200)) {
            seen.push(line);
        }
        let working = seen.iter().any(|l| l.starts_with("emitter scanner: ") && !l.contains("0 line(s)"))
            && seen.iter().any(|l| l.starts_with("opc plc: opc.tcp://127.0.0.1:"))
            && seen.iter().any(|l| l.starts_with("channel machine: 1 peer(s)") && l.contains(" announced, ") && !l.contains("controller: 0 announced"))
            && seen.iter().any(|l| l.starts_with("channel unit: connected to "));
        if working {
            break;
        }
    }
    assert_eq!(seen[0], format!("{}: 4 part(s) running; close stdin or end the process to stop", path.display()), "{seen:?}");
    assert!(seen.iter().any(|l| l.starts_with("emitter scanner: ") && !l.contains("0 line(s)")), "{seen:?}");
    assert!(seen.iter().any(|l| l.starts_with("channel machine: 1 peer(s)") && !l.contains("controller: 0 announced")), "{seen:?}");
    child.stdin.take().unwrap().write_all(b"stop\n").unwrap();
    let status = child.wait().unwrap();
    assert!(status.success(), "{status:?}");
    let rest: Vec<String> = rx.try_iter().collect();
    assert_eq!(rest.last().map(String::as_str), Some("stopped"), "{rest:?}");
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(), "the port is free again");
    assert!(std::fs::read_dir(dir.join("logs")).unwrap().count() >= 1, "the scanner wrote its file");
    // A second run, stopped by stdin closing with nothing written.
    let mut child = Command::new(exe).arg(&path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let started = Instant::now();
    let mut running = false;
    while started.elapsed() < Duration::from_secs(15) {
        if let Ok(line) = rx.recv_timeout(Duration::from_millis(200))
            && line.starts_with("channel unit: connected to ")
        {
            running = true;
            break;
        }
    }
    assert!(running);
    drop(child.stdin.take());
    let status = child.wait().unwrap();
    assert!(status.success(), "{status:?}");
    let rest: Vec<String> = rx.try_iter().collect();
    assert_eq!(rest.last().map(String::as_str), Some("stopped"), "stdin closed: {rest:?}");
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_reference_is_the_text_this_test_holds() {
    let actual = paragraphs(&doc("bench-schema.md"));
    let expected = paragraphs(REFERENCE);
    for (i, (a, e)) in actual.iter().zip(expected.iter()).enumerate() {
        assert_eq!(a, e, "paragraph {i} of docs/bench-schema.md differs from the copy this test holds");
    }
    assert_eq!(actual.len(), expected.len(), "docs/bench-schema.md has {} paragraphs, the copy {}", actual.len(), expected.len());
}

/// The reference, `docs/bench-schema.md`, as this test holds it: its paragraphs, the TOML block
/// left out. An edit to the reference is an edit here, beside its pin.
const REFERENCE: &str = r#"
# The bench file: `veredas-bench`, version 1

A bench is the plant a consumer meets before the real one (`docs/veredas-plan.md`,
"veredas-maquinar: the bench"): the log emitters, the OPC UA, Modbus and EtherNet/IP servers and
the telegram channels it talks to, each configured, none programmed. One bench file lists them all by name, and the
device's own files sit beside it: core's channel file (`docs/channel-schema.md`), the catalog
(`docs/catalog-schema.md`) and, when a controller rides a channel, its rules file
(`docs/controller-schema.md`). `veredas_maquinar::bench::load` reads the bench file and every
file it points at and checks each part as its own module checks it, so a bench file that loads
is a bench every part of which can start; `Bench::start` then runs every part behind one handle,
emitters first, then servers (OPC UA, then Modbus, then EtherNet/IP), then channels, each kind in
the file's order
(TOML groups a kind's tables under one key, so a file keeps no order across kinds), and the executable
`veredas-maquinar <bench.toml>` runs it from the command line (`--check` reads, checks and
prints the plan without starting anything). No device is named anywhere: a
plant's bench is a folder of files like the ones below, kept outside the suite's code.

Every plain-text config and project file of Veredas 2 follows this pattern (`docs/questions.md`
Q10, "TOML, suite-wide"): TOML, a `format` and a `version` first, documented defaults, a
misspelled key refused, an unknown format or version refused rather than guessed.

## The worked example: a scanner's log, a controller's OPC UA server, and a machine talking to its unit

A scanner writes a line every 200 ms into a daily file that rotates every 1000 lines; a
controller serves two tags over OPC UA on a port the machine chooses, a speed drawn between 1.0
and 2.0 every 500 ms and a run flag a client may write, and reacts to the flag written true by
setting the speed to 2.0 after 100 ms; and the machine of the controller file's worked example
listens on port 5020 for the unit of that example, which dials it. The channel file, the
catalog and the two rules files are those of the channel and controller references, written
beside this file under the names it gives; the emitter writes under `logs`, beside it too.
`crates/veredas-maquinar/tests/bench_file.rs` writes those files, loads this block, starts it,
sees every part do its work, and runs the executable on it; it holds a copy of this text, so an
edit here is an edit there.

## A second example: a drive on Modbus/TCP

A drive answers as unit 1 on port 5502: a client writes its speed setpoint into a holding register
and starts it with a coil; the plant side answers with the speed reached, a float over two input
registers, and a ready input. `crates/veredas-maquinar/tests/modbus_server.rs` loads this block on
a port the machine chooses, drives it with a Modbus client the bench did not write, and sees the
plant side meet it.


## A third example: an I/O block on EtherNet/IP

An I/O block answers on the standard ports: a scanner opens a Class 1 connection naming its
configuration, output and input assemblies, writes a start bit at its RPI, and reads back a ready
bit and a speed the plant side sets. `crates/veredas-maquinar/tests/ethernetip_adapter.rs` loads this
block on ports the machine chooses, opens the connection with codecs the bench did not write, and
sees the plant side meet it.

## Keys

Each key states in parentheses that it is required, or the value it takes when left out. Every
part has a name, exact, with no surrounding whitespace, and no two parts share one. A path is
relative to the bench file's folder.

**The top level**.
- `format` (required): always `"veredas-bench"`.
- `version` (required): this reader knows `1`. A file of another format or an unknown version is
  refused, never guessed.
- `seed` (none): the seed of every part's draws, for a run that repeats: each emitter without a
  seed of its own draws from this seed plus its index, each channel and its controller from
  this seed; the clock when absent.
- `out_dir` (`"."`): where the emitters write, created when there is an emitter.
- `emitter` (none): `[[emitter]]` tables.
- `opc` (none): `[[opc]]` tables.
- `modbus` (none): `[[modbus]]` tables.
- `ethernetip` (none): `[[ethernetip]]` tables.
- `channel` (none): `[[channel]]` tables.

**An emitter** (`[[emitter]]`): a log a plant would write, line by line into rotating files.
- `name` (required): the part's name.
- `template` (required): the line, `{ts}` the timestamp and `{name}` a value, anything else
  verbatim.
- `ts_format` (required): the `chrono` format `{ts}` renders with.
- `values` (none): the values drawn fresh for every line, as inline tables.
- `interval_ms` (required): milliseconds between lines; `0` writes as fast as it can.
- `file` (required): the file name, `chrono` specifiers and `{n}` for the rotation index; a name
  that is empty, has a path separator or is reserved by the platform is refused.
- `max_lines` (`0`): rotate to the next index after this many lines; `0` never rotates on
  count.
- `encoding` (`"utf-8"`): `"utf-8"` or `"utf-16le"`, the latter with a byte-order mark.
- `seed` (none): a fixed seed replays the same values; absent, the bench's seed plus the
  emitter's index, else the clock.

**A value** (an entry of `values`): drawn within `[low, high]`, rendered to `decimals`.
- `name` (required): the placeholder's name.
- `low` (required), `high` (required): finite, `low` not above `high`.
- `decimals` (`0`): the places rendered, at most 15.

**An OPC UA server** (`[[opc]]`): a controller's tags on a real endpoint.
- `name` (required): the part's name.
- `bind` (`"127.0.0.1"`): the address the server binds and advertises, never `0.0.0.0`.
- `port` (`4855`): `0` lets the machine choose a free port.
- `user` (none): a user the server accepts by name and password; anonymous is always accepted.
- `max_nodes_per_read` (none): the most nodes one read may carry; absent, the stack's own limit.
- `max_sessions` (none): the most sessions held at once.
- `tags` (none): the tags, as inline tables.
- `reactions` (none): the reactions, as inline tables.

**A user** (the value of `user`).
- `name` (required), `password` (required): shown in no status or log.

**A tag** (an entry of `tags`).
- `id` (required): the string identifier in the bench's namespace; the node id is
  `ns=<index>;s=<id>`.
- `name` (`id`): the browse and display name a client sees.
- `type` (required): `int` (an Int32), `float` (a Double), `bool` or `text`.
- `access` (`"read"`): `read`, `write` (a command the controller takes but does not report) or
  `read-write`.
- `value` (the type's zero): the initial value as text, parsed per the type.
- `randomize` (none): a fresh value within a range on an interval; a text tag takes none.
- `ramp` (none): a value stepping through a range on an interval and wrapping, for plant
  motion (a place along a loop, a level filling); an int or a float, never with `randomize`.

**A draw** (the value of `randomize`).
- `low` (`0`), `high` (`0`): the range; an int draws the whole numbers in it, a bool flips.
- `decimals` (`0`): a float's places.
- `interval_ms` (`1000`): between draws; at least the server's tick of 20 ms.

**A ramp** (the value of `ramp`): each step adds `step` to what the tag holds, wrapped by the
range's width while past `high`; a value outside the range (a client's write, a plant-side set)
starts again at `low`, one inside it moves the ramp on from there.
- `low` (required), `high` (required): the range, finite, `low` below `high`, its width finite;
  the tag's initial value inside it.
- `step` (`1`): above 0 and at most the range's width (with places, counted in their units); an
  int's range and step are whole numbers.
- `decimals` (none): a float's places, which a float ramp names (an int takes none): every step is
  counted in units of the last place, so the ramp reaches its high exactly, and `low`, `high` and
  `step` must be whole numbers of those units, small enough to count exactly (at most 2^51 of
  them: with fifteen places, a range within ±2.25).
- `interval_ms` (`1000`): between steps; at least the server's tick of 20 ms.

**A reaction** (an entry of `reactions`): what the plant does when a client writes a tag.
- `on_write` (required): the tag watched.
- `equals` (none): the value that fires it, as text parsed per the tag's type; absent, any
  change fires it. Either way only what a client wrote or the plant side set is a change, never a
  draw, a ramp's step or a reaction's step; a float holding NaN is no change from NaN.
- `tag` (`on_write`): the tag the steps set.
- `then` (required): the steps, as inline tables, each at its delay from the moment of firing
  (steps falling due in one tick land in the order of their delays).

**A step** (an entry of `then`).
- `after_ms` (`0`): the delay.
- `set` (required): the value set, as text parsed per the target tag's type.

**A Modbus server** (`[[modbus]]`): a device's four tables over Modbus/TCP: coils and discrete
inputs (bits), input and holding registers (16-bit words), a client writing the coils and the
holding registers only.
- `name` (required): the part's name.
- `bind` (`"127.0.0.1"`): the address the server listens on.
- `port` (`502`): the protocol's port; on Linux a port below 1024 needs a privilege, so a bench
  there names another; `0` lets the machine choose a free port.
- `unit` (none): the unit identifier the device plays, a request to another unit answered with
  exception 0B as a gateway answers for a device that is not there, 0 and 255 included (the ones a
  client addressing a device on TCP directly sends: leave `unit` out for such a device); absent,
  every unit is the device.
- `max_connections` (none): the most connections held at once; one more is accepted and closed at
  once.
- `coils` (none), `discrete_inputs` (none), `input_registers` (none), `holding_registers` (none):
  a table's size, at most 65536, its addresses 0 to the size less one; absent, the table reaches
  the end of its last point, and holds nothing without one.
- `points` (none): the points, as inline tables.

**A point** (an entry of `points`): a named value in one of the tables; no two points of a table
share an address.
- `name` (required): the name the bench's handle reads and sets it by.
- `table` (required): `coil`, `discrete-input`, `input-register` or `holding-register`.
- `address` (required): the protocol's address, from 0, as a request carries it.
- `type` (`bool` in a bit table, `u16` in a register table): `bool` in a coil or a discrete input;
  `u16`, `i16`, `u32`, `i32` or `f32` (a single-precision float) in a register table, the last
  three over two registers.
- `words` (`"high-first"`): which of a two-register point's registers holds the high word,
  `high-first` (the one at its address) or `low-first`; a point of one register or a bit takes
  none. A register's own two bytes are always high byte first, as the protocol sends them.
- `value` (the type's zero): the initial value as text, parsed per the type.

**An EtherNet/IP adapter** (`[[ethernetip]]`): a device's assemblies over EtherNet/IP, explicit
messages on a TCP session and Class 1 cyclic I/O over UDP.
- `name` (required): the part's name.
- `bind` (`"127.0.0.1"`): the address the adapter listens on.
- `port` (`44818`): the encapsulation port, TCP for sessions and UDP for a ListIdentity; `0` lets
  the machine choose, the same port for both.
- `io_port` (`2222`): the Class 1 port, UDP; `0` lets the machine choose, the port then given in
  every Forward_Open's reply.
- `identity` (the defaults below): what ListIdentity and the Identity object report.
- `rpi_min_us` (`1000`), `rpi_max_us` (`10000000`): the RPIs accepted, in microseconds.
- `max_sessions` (none): the most sessions registered at once.
- `max_connections` (none): the most Class 1 connections open at once.
- `assemblies` (none): the assemblies, as inline tables.
- `connections` (none): the Class 1 connections the adapter accepts, as inline tables.
- `points` (none): named values in the assemblies, as inline tables.

**An identity** (the value of `identity`).
- `vendor_id` (`0`), `device_type` (`12`), `product_code` (`0`): the Identity object's first three
  attributes, 12 a communications adapter.
- `revision` (`"1.1"`): `major.minor`, the major 1 to 127, the minor 1 to 255.
- `serial` (`0`): the serial number.
- `product_name` (`"veredas bench adapter"`): at most 32 ASCII characters.

**An assembly** (an entry of `assemblies`): an instance of the Assembly object, its data a run of
bytes, zeros at the start.
- `instance` (required): 1 to 65535, no two alike.
- `size` (required): its bytes, at most 65000.

**A connection** (an entry of `connections`): a Class 1 connection the adapter accepts, by the
three assemblies its Forward_Open's path names; the size each direction states counts the data,
the 16-bit sequence count, and the run/idle header when the direction carries one.
- `config` (required): the configuration assembly, named first; a Forward_Open's data segment is
  written into it.
- `output` (required): the assembly the adapter consumes (the originator's outputs, O→T).
- `input` (required): the assembly the adapter produces (the originator's inputs, T→O).
- `output_header` (`"run-idle"`): what the consumed data carries before its bytes: `run-idle` (a
  32-bit run/idle header) or `modeless`.
- `input_header` (`"modeless"`): the same for the produced data.

**An assembly point** (an entry of `points` of an adapter): a named value in an assembly's bytes,
little-endian as CIP carries it; no two points of an assembly share a byte, unless both are bools
on different bits.
- `name` (required): the name the bench's handle reads and sets it by.
- `assembly` (required): the assembly's instance.
- `byte` (required): the offset of its first byte.
- `bit` (none): a bool's bit, 0 to 7; a bool names one and no other type takes one.
- `type` (`bool` with a bit, `u8` without): `bool`, `u8`, `i8`, `u16`, `i16`, `u32`, `i32` or `f32`.
- `value` (the type's zero): the initial value as text, parsed per the type.

**A channel** (`[[channel]]`): one telegram link, listening or dialling.
- `name` (required): the part's name.
- `role` (required): `listen` (wait for the peer to connect) or `connect` (dial it).
- `host` (`"127.0.0.1"`): the address to bind, or the host to dial.
- `port` (required): the port to bind or dial.
- `channel` (required): the path of core's channel file.
- `catalog` (required): the path of the catalog file; its ids must ride in the channel file's
  framing, as the catalog's own check says (`docs/catalog-schema.md`, `id_field`).
- `controller` (none): the path of the controller rules file, read against the catalog; none
  for a channel that only sends its own messages.
- `messages` (none): the channel's own messages, as inline tables.

**A message** (an entry of `messages`): a message of the catalog the channel sends on an
interval, or on demand.
- `message` (required): its name in the catalog.
- `every_ms` (`0`): the interval; `0` is on demand only.
- `fields` (none): its fields, one for every name the message writes, each a field of the
  controller file's reference (`docs/controller-schema.md`, "A field") with the sources a plain
  channel has: no `from`, no `register`, no `decide`.

A misspelled key in any table, a part without a name or with a name another part has, an
emitter's file name or timestamp format that is bad, a value's range that is empty or not
finite, anything the OPC UA or the Modbus server's own checks refuse (a point over another or past
its table's end, a type its table cannot hold, a value that does not parse) or the EtherNet/IP
adapter's (an assembly declared twice, a connection naming an assembly there is not, a point past
its assembly's bytes or over another), a file a channel points at that cannot be read or does not
load, an `id_field` that does not ride in the framing, a controller file that
does not load against the catalog, a message the catalog lacks or whose fields leave a name
unwritten or use a source a plain channel lacks, is refused when the bench file loads, with the
place named, before anything binds, opens or spawns. A part that cannot start (a port taken, a
folder that cannot be created) stops the parts started before it and is named.
"#;
