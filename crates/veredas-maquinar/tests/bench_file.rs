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
use veredas_maquinar::opc::{Access, Credentials, Randomize, Reaction, Step, TagConfig, TagType};

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
         tags = [ {{ id = \"t\", name = \"T\", type = \"float\", access = \"read-write\", value = \"1\", randomize = {{ low = 1, high = 2, decimals = 1, interval_ms = 100 }} }} ]\n\
         reactions = [ {{ on_write = \"t\", equals = \"5\", tag = \"t\", then = [ {{ after_ms = 1, set = \"6\" }} ] }} ]\n\n\
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
        ["The top level", "An emitter", "A value", "An OPC UA server", "A user", "A tag", "A draw", "A reaction", "A step", "A channel", "A message"],
        "the reference's eleven tables"
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
            "A channel" => "[[channel]]",
            _ => return None,
        })
    };
    let inline_full = |table: &str| -> &'static str {
        match table {
            "A value" => "name = \"v\"\nlow = 1\nhigh = 2\ndecimals = 1\n",
            "A user" => "name = \"u\"\npassword = \"p\"\n",
            "A tag" => "id = \"t\"\nname = \"T\"\ntype = \"float\"\naccess = \"read-write\"\nvalue = \"1\"\nrandomize = { low = 1, high = 2 }\n",
            "A draw" => "low = 1\nhigh = 2\ndecimals = 1\ninterval_ms = 100\n",
            "A reaction" => "on_write = \"t\"\nequals = \"5\"\ntag = \"t\"\nthen = [ { set = \"6\" } ]\n",
            "A step" => "after_ms = 1\nset = \"6\"\n",
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
            "A channel" => full.replace("[[channel]]\n", &format!("[[channel]]\n{zz}")),
            "A value" => full.replace("{ name = \"v\",", "{ name = \"v\", zz = 1,"),
            "A user" => full.replace("{ name = \"u\",", "{ name = \"u\", zz = 1,"),
            "A tag" => full.replace("{ id = \"t\",", "{ id = \"t\", zz = 1,"),
            "A draw" => full.replace("randomize = { low = 1,", "randomize = { low = 1, zz = 1,"),
            "A reaction" => full.replace("{ on_write = \"t\",", "{ on_write = \"t\", zz = 1,"),
            "A step" => full.replace("{ after_ms = 1,", "{ after_ms = 1, zz = 1,"),
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
                ("A tag", _) => check!(TagConfig, |_| false),
                ("A draw", "low") => check!(Randomize, |r| default == "0" && r.low == 0.0),
                ("A draw", "high") => check!(Randomize, |r| default == "0" && r.high == 0.0),
                ("A draw", "decimals") => check!(Randomize, |r| default == "0" && r.decimals == 0),
                ("A draw", "interval_ms") => check!(Randomize, |r| default == "1000" && r.interval_ms == 1000),
                ("A reaction", "equals") => check!(Reaction, |r| default == "none" && r.equals.is_none()),
                ("A reaction", "tag") => check!(Reaction, |r| default == "on_write" && r.tag.is_none()),
                ("A reaction", _) => check!(Reaction, |_| false),
                ("A step", "after_ms") => check!(Step, |s| default == "0" && s.after_ms == 0),
                ("A step", _) => check!(Step, |_| false),
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
    assert!(prose.contains("A misspelled key in any table, a part without a name or with a name another part has, an emitter's file name or timestamp format that is bad, a value's range that is empty or not finite, anything the OPC UA server's own checks refuse, a file a channel points at that cannot be read or does not load, an `id_field` that does not ride in the framing, a controller file that does not load against the catalog, a message the catalog lacks or whose fields leave a name unwritten or use a source a plain channel lacks, is refused when the bench file loads, with the place named, before anything binds, opens or spawns."));
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
    assert!(doc_prose().contains("emitters first, then servers, then channels, each kind in the file's order (TOML groups a kind's tables under one key, so a file keeps no order across kinds)"));
    let dir = scratch("order");
    let (_, port) = plant(&dir);
    let text = format!(
        "{}[[opc]]\nname = \"first\"\nport = 0\n\n[[channel]]\nname = \"second\"\nrole = \"listen\"\nport = {port}\nchannel = \"channel.toml\"\ncatalog = \"catalog.toml\"\n\n[[emitter]]\nname = \"third\"\ntemplate = \"{{ts}}\"\nts_format = \"%H\"\ninterval_ms = 100\nfile = \"t.log\"\n\n[[emitter]]\nname = \"fourth\"\ntemplate = \"{{ts}}\"\nts_format = \"%H\"\ninterval_ms = 100\nfile = \"f.log\"\n\n[[opc]]\nname = \"fifth\"\nport = 0\n",
        header()
    );
    let spec = load(&text, &dir).unwrap();
    let order = |lines: &[String]| lines.iter().map(|l| l.split('`').nth(1).unwrap_or("").to_string()).collect::<Vec<_>>();
    assert_eq!(order(&spec.plan()), ["third", "fourth", "first", "fifth", "second"]);
    let bench = Bench::start(spec).unwrap();
    assert_eq!(bench.status().iter().map(|(n, _)| n.clone()).collect::<Vec<_>>(), ["third", "fourth", "first", "fifth", "second"]);
    assert_eq!(bench.lines().iter().map(|l| l.split(' ').nth(1).unwrap().trim_end_matches(':').to_string()).collect::<Vec<_>>(), ["third", "fourth", "first", "fifth", "second"]);
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
    assert_eq!((out.status.code(), String::from_utf8_lossy(&out.stderr).trim()), (Some(1), "usage: veredas-maquinar <bench.toml> [--check]"));
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
"veredas-maquinar: the bench"): the log emitters, the OPC UA servers and the telegram channels
it talks to, each configured, none programmed. One bench file lists them all by name, and the
device's own files sit beside it: core's channel file (`docs/channel-schema.md`), the catalog
(`docs/catalog-schema.md`) and, when a controller rides a channel, its rules file
(`docs/controller-schema.md`). `veredas_maquinar::bench::load` reads the bench file and every
file it points at and checks each part as its own module checks it, so a bench file that loads
is a bench every part of which can start; `Bench::start` then runs every part behind one handle,
emitters first, then servers, then channels, each kind in the file's order (TOML groups a
kind's tables under one key, so a file keeps no order across kinds), and the executable
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

**A draw** (the value of `randomize`).
- `low` (`0`), `high` (`0`): the range; an int draws the whole numbers in it, a bool flips.
- `decimals` (`0`): a float's places.
- `interval_ms` (`1000`): between draws; at least the server's tick of 20 ms.

**A reaction** (an entry of `reactions`): what the plant does when a client writes a tag.
- `on_write` (required): the tag watched.
- `equals` (none): the value that fires it, as text parsed per the tag's type; absent, any
  change fires it.
- `tag` (`on_write`): the tag the steps set.
- `then` (required): the steps, as inline tables, each at its delay from the moment of firing.

**A step** (an entry of `then`).
- `after_ms` (`0`): the delay.
- `set` (required): the value set, as text parsed per the target tag's type.

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
finite, anything the OPC UA server's own checks refuse, a file a channel points at that cannot be
read or does not load, an `id_field` that does not ride in the framing, a controller file that
does not load against the catalog, a message the catalog lacks or whose fields leave a name
unwritten or use a source a plain channel lacks, is refused when the bench file loads, with the
place named, before anything binds, opens or spawns. A part that cannot start (a port taken, a
folder that cannot be created) stops the parts started before it and is named.
"#;
