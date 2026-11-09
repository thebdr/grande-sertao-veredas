//! OPC UA sources (contract: the OPC sources clause): a source's configuration compiled into
//! core's poll spec before anything connects, every mistake refused by name; the worker polling
//! the bench's OPC UA server from a bench file into typed rows with bits carved out, on-change
//! rules keeping transitions, the identity from a named entry of the secrets file, and the writes
//! gated by the master switch and the whitelist, proven on the server side.

use std::path::Path;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use veredas_core::opc::error::NodeIdError;
use veredas_core::opc::spec::Auth;
use veredas_maquinar::bench::{self, Bench};
use veredas_portal::opc::{
    ConnState, DedupState, OpcBitConfig, OpcError, OpcEvent, OpcRuleConfig, OpcSourceConfig, OpcTagConfig, WriteError, WriteGate,
    check_name, compile, map_poll_dedup, parse_type, spawn, table_of,
};
use veredas_portal::{SeriesType, Value};

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

/// The plant: an OPC UA server from a bench file, on a free port, with a user, a drawn speed, a
/// run flag with a reaction, a packed word, a text tag and a write-only command.
fn plant(dir: &Path) -> Bench {
    let text = "format = \"veredas-bench\"\nversion = 1\nseed = 3\n\n[[opc]]\nname = \"plc\"\nport = 0\nuser = { name = \"operator\", password = \"s3cret\" }\ntags = [\n  { id = \"speed\", type = \"float\", value = \"1.5\", randomize = { low = 1.0, high = 2.0, decimals = 2, interval_ms = 100 } },\n  { id = \"run\", type = \"bool\", access = \"read-write\" },\n  { id = \"word\", type = \"int\", value = \"5\", access = \"read-write\" },\n  { id = \"label\", type = \"text\", value = \"idle\", access = \"read-write\" },\n  { id = \"cmd\", type = \"int\", access = \"write\" },\n]\nreactions = [ { on_write = \"run\", equals = \"true\", tag = \"word\", then = [ { after_ms = 50, set = \"6\" } ] } ]\n";
    std::fs::write(dir.join("bench.toml"), text).unwrap();
    Bench::start(bench::load(text, dir).unwrap()).unwrap()
}

fn tag(node: &str, series: &str, ty: &str) -> OpcTagConfig {
    OpcTagConfig { node_id: node.into(), series: series.into(), ty: ty.into(), writable: false, bits: vec![] }
}

/// The source's configuration against the plant's namespace.
fn config(bench: &Bench, user: Option<&str>, entry: Option<&str>) -> OpcSourceConfig {
    let plc = bench.server("plc").unwrap();
    let node = |id: &str| plc.node_id_str(id);
    OpcSourceConfig {
        source_id: "plc".into(),
        display_name: "".into(),
        endpoint_url: plc.endpoint_url.clone(),
        user: user.map(String::from),
        secret: entry.map(String::from),
        poll_ms: 50,
        rules: vec![
            OpcRuleConfig {
                name: "plc_main".into(),
                store_on_change: false,
                tags: vec![
                    tag(&node("speed"), "speed", "float"),
                    tag(&node("run"), "run", "int"),
                    OpcTagConfig {
                        node_id: node("word"),
                        series: "word".into(),
                        ty: "int".into(),
                        writable: false,
                        bits: vec![
                            OpcBitConfig { bit: 0, width: 1, series: "b0".into() },
                            OpcBitConfig { bit: 1, width: 2, series: "mid".into() },
                            OpcBitConfig { bit: 2, width: 0, series: "b2".into() },
                        ],
                    },
                    OpcTagConfig { node_id: node("cmd"), series: "cmd".into(), ty: "int".into(), writable: true, bits: vec![] },
                    OpcTagConfig { node_id: node("run"), series: "run_cmd".into(), ty: "int".into(), writable: true, bits: vec![] },
                ],
            },
            OpcRuleConfig {
                name: "plc_text".into(),
                store_on_change: true,
                tags: vec![tag(&node("label"), "label", "text")],
            },
        ],
    }
}

fn no_secret(_: &str) -> Option<String> {
    None
}

fn secret(entry: &str) -> Option<String> {
    (entry == "plc_pw").then(|| "s3cret".to_string())
}

/// One poll as mapped: the index clock, the arrival clock, one row per rule kept, which rules, the failures.
type Poll = (i64, i64, Vec<Vec<Option<Value>>>, Vec<usize>, Vec<(String, String)>);

/// Drain the worker's events, keeping the states and the polls as mapped rows per rule.
struct Seen {
    states: Vec<ConnState>,
    polls: Vec<Poll>,
}

fn drain(rx: &Receiver<OpcEvent>, spec: &veredas_core::opc::spec::PollSpec, dedup: &mut DedupState, seq: &mut u64, seen: &mut Seen) {
    while let Ok(ev) = rx.try_recv() {
        match ev {
            OpcEvent::Status { source_id, state } => {
                assert_eq!(source_id, "plc");
                seen.states.push(state);
            }
            OpcEvent::Poll { source_id, index_ms, arrival_ms, values } => {
                assert_eq!(source_id, "plc");
                *seq += 1;
                let mapped = map_poll_dedup(spec, &values, index_ms, *seq, Some(dedup));
                let rows: Vec<Vec<Option<Value>>> = mapped.rows.iter().map(|r| r.values.clone()).collect();
                let which: Vec<usize> = mapped.rows.iter().map(|r| r.rule_index).collect();
                seen.polls.push((index_ms, arrival_ms, rows, which, mapped.failures));
            }
        }
    }
}

#[test]
fn the_bench_s_server_is_polled_into_typed_rows_with_bits_and_on_change_kept() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let plc = bench.server("plc").unwrap();
    let cfg = config(&bench, None, None);
    let spec = compile(&cfg, &no_secret).unwrap();
    assert_eq!(spec.display_name, "plc", "the id when no display name is given");
    assert_eq!(spec.auth, Auth::Anonymous);
    assert_eq!(spec.poll, Duration::from_millis(50));
    assert_eq!(spec.poll_nodes().len(), 5, "run is polled once though two tags name it");
    let (name, columns) = table_of(&spec.rules[0]);
    assert_eq!(name, "plc_main");
    assert_eq!(
        columns,
        vec![
            ("speed".to_string(), SeriesType::Float),
            ("run".to_string(), SeriesType::Int),
            ("word".to_string(), SeriesType::Int),
            ("b0".to_string(), SeriesType::Int),
            ("mid".to_string(), SeriesType::Int),
            ("b2".to_string(), SeriesType::Int),
            ("cmd".to_string(), SeriesType::Int),
            ("run_cmd".to_string(), SeriesType::Int),
        ]
    );
    assert_eq!(table_of(&spec.rules[1]), ("plc_text".to_string(), vec![("label".to_string(), SeriesType::Text)]));

    let (tx, rx) = std::sync::mpsc::channel();
    let worker = spawn(spec.clone(), tx, WriteGate::new(false)).unwrap();
    let mut seen = Seen { states: Vec::new(), polls: Vec::new() };
    let mut dedup = DedupState::default();
    let mut seq = 0u64;
    assert!(wait(Duration::from_secs(10), || {
        drain(&rx, &spec, &mut dedup, &mut seq, &mut seen);
        seen.polls.len() >= 10
    }), "{} polls", seen.polls.len());
    assert_eq!(&seen.states[..2], [ConnState::Connecting, ConnState::Connected]);

    // The first poll: both rules, every column typed, the bits carved out of the word 5.
    let (index_ms, arrival_ms, rows, which, failures) = &seen.polls[0];
    assert_eq!(which, &[0, 1]);
    assert_eq!(failures.iter().map(|(c, _)| c.as_str()).collect::<Vec<_>>(), ["cmd"], "the write-only command cannot be read: a hole, reported");
    assert!(failures[0].1.contains("Bad"), "{:?}", failures[0]);
    let main = &rows[0];
    assert!(matches!(main[0], Some(Value::Float(v)) if (1.0..=2.0).contains(&v)), "{main:?}");
    assert_eq!(main[1], Some(Value::Int(0)), "a false flag is 0");
    assert_eq!(main[2], Some(Value::Int(5)));
    assert_eq!(main[3], Some(Value::Int(1)), "bit 0 of 5");
    assert_eq!(main[4], Some(Value::Int(2)), "bits 1 and 2 of 5 (0b10)");
    assert_eq!(main[5], Some(Value::Int(1)), "bit 2 of 5, width 0 read as 1");
    assert_eq!(main[6], None, "the write-only command is a hole");
    assert_eq!(main[7], Some(Value::Int(0)));
    assert_eq!(rows[1], vec![Some(Value::Text("idle".into()))]);
    // Two clocks: the index on the plant's local axis, the arrival on the unix clock.
    let now = veredas_portal::now_unix_ms();
    assert!(*arrival_ms > now - 60_000 && *arrival_ms <= now + 1);
    let expected = chrono::Local::now().offset().local_minus_utc() as i64 * 1000;
    assert!((*index_ms - *arrival_ms - expected).abs() < 5_000, "{index_ms} {arrival_ms}");
    // The on-change rule yields no row while the label holds; the main rule a row per poll.
    let text_rows = seen.polls.iter().filter(|(_, _, _, w, _)| w.contains(&1)).count();
    assert_eq!(text_rows, 1, "the text rule's first poll alone");
    assert!(seen.polls.iter().all(|(_, _, _, w, _)| w.contains(&0)));
    // The draw moves the speed between polls.
    let speeds: std::collections::BTreeSet<String> = seen.polls.iter().map(|(_, _, r, _, _)| format!("{:?}", r[0][0])).collect();
    assert!(speeds.len() >= 2, "{speeds:?}");
    // The plant changes the label: a transition, one row; then the heartbeat at the 60th quiet poll.
    plc.set("label", "running");
    let before = seen.polls.len();
    assert!(wait(Duration::from_secs(5), || {
        drain(&rx, &spec, &mut dedup, &mut seq, &mut seen);
        seen.polls[before..].iter().any(|(_, _, r, w, _)| w.contains(&1) && r.last() == Some(&vec![Some(Value::Text("running".into()))]))
    }));
    let text_rows_now = seen.polls.iter().filter(|(_, _, _, w, _)| w.contains(&1)).count();
    assert_eq!(text_rows_now, 2);
    assert!(wait(Duration::from_secs(10), || {
        drain(&rx, &spec, &mut dedup, &mut seq, &mut seen);
        seen.polls.iter().filter(|(_, _, _, w, _)| w.contains(&1)).count() >= 3
    }), "the heartbeat row");
    let text_polls: Vec<u64> = seen
        .polls
        .iter()
        .enumerate()
        .filter(|(_, (_, _, _, w, _))| w.contains(&1))
        .map(|(i, _)| i as u64 + 1)
        .collect();
    assert_eq!(text_polls[2] - text_polls[1], 60, "the heartbeat is the 60th poll after the last kept: {text_polls:?}");
    // The plant's run flag written by the plant side reads as 1 and fires the reaction on the word.
    plc.set("run", true);
    assert!(wait(Duration::from_secs(5), || {
        drain(&rx, &spec, &mut dedup, &mut seq, &mut seen);
        seen.polls.last().is_some_and(|(_, _, r, _, _)| r[0][1] == Some(Value::Int(1)) && r[0][2] == Some(Value::Int(6)))
    }), "{:?}", seen.polls.last());
    let last = &seen.polls.last().unwrap().2[0];
    assert_eq!((&last[3], &last[4], &last[5]), (&Some(Value::Int(0)), &Some(Value::Int(3)), &Some(Value::Int(1))), "the bits of 6");
    worker.handle.join().unwrap();
    bench.stop();
}

#[test]
fn writes_are_gated_by_the_switch_and_the_whitelist_and_land_on_the_server_side() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    let plc = bench.server("plc").unwrap();
    let cfg = config(&bench, None, None);
    let spec = compile(&cfg, &no_secret).unwrap();
    let gate = WriteGate::new(false);
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = spawn(spec.clone(), tx, gate.clone()).unwrap();
    let mut seen = Seen { states: Vec::new(), polls: Vec::new() };
    let mut dedup = DedupState::default();
    let mut seq = 0u64;
    assert!(wait(Duration::from_secs(10), || {
        drain(&rx, &spec, &mut dedup, &mut seq, &mut seen);
        seen.states.contains(&ConnState::Connected)
    }));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let cmd = plc.node_id_str("cmd");
    let write = |node: &str, v: opcua_types::Variant| rt.block_on(worker.writer.write_tags(vec![(node.to_string(), v)]));
    // Disarmed: refused before anything is sent; the plant untouched.
    assert_eq!(write(&cmd, opcua_types::Variant::Int32(7)), Err(WriteError::Disarmed));
    assert!(!worker.writer.is_armed());
    assert_eq!(plc.get("cmd"), Some(opcua_types::Variant::Int32(0)));
    // Armed: the whitelist. A tag not marked writable, and a tag the spec lacks, answer alike.
    gate.set(true);
    assert!(worker.writer.is_armed());
    let speed = plc.node_id_str("speed");
    assert_eq!(
        write(&speed, opcua_types::Variant::Double(9.0)),
        Err(WriteError::NotWritable { source_id: "plc".into(), tag: speed.clone() })
    );
    let ghost = plc.node_id_str("ghost");
    assert_eq!(write(&ghost, opcua_types::Variant::Int32(1)), Err(WriteError::NotWritable { source_id: "plc".into(), tag: ghost }));
    assert_eq!(worker.writer.writable_type(&cmd), Some(SeriesType::Int));
    assert_eq!(worker.writer.writable_type(&speed), None);
    // A permitted write lands where the server side reads it.
    let statuses = write(&cmd, opcua_types::Variant::Int32(7)).unwrap();
    assert_eq!(statuses.len(), 1);
    assert!(statuses[0].is_good(), "{statuses:?}");
    assert!(wait(Duration::from_secs(2), || plc.get("cmd") == Some(opcua_types::Variant::Int32(7))));
    // A write of the wrong wire type is refused by the plant in place, with its status.
    let statuses = write(&cmd, opcua_types::Variant::Double(1.0)).unwrap();
    assert!(!statuses[0].is_good(), "{statuses:?}");
    assert_eq!(plc.get("cmd"), Some(opcua_types::Variant::Int32(7)));
    // The run flag written true fires the plant's reaction, seen in the next polls.
    let run = plc.node_id_str("run");
    assert!(write(&run, opcua_types::Variant::Boolean(true)).unwrap()[0].is_good());
    assert!(wait(Duration::from_secs(5), || {
        drain(&rx, &spec, &mut dedup, &mut seq, &mut seen);
        seen.polls.last().is_some_and(|(_, _, r, _, _)| r[0][2] == Some(Value::Int(6)))
    }));
    assert!(plc.status().reactions_fired >= 1);
    // Disarmed again, live: refused again.
    gate.set(false);
    assert_eq!(write(&cmd, opcua_types::Variant::Int32(8)), Err(WriteError::Disarmed));
    assert_eq!(plc.get("cmd"), Some(opcua_types::Variant::Int32(7)));
    worker.handle.join().unwrap();
    bench.stop();
}

#[test]
fn the_identity_comes_from_a_named_secret_and_a_wrong_one_never_connects() {
    let dir = tempfile::tempdir().unwrap();
    let bench = plant(dir.path());
    // The entry is named apart from the source id and the user, so a lookup by either fails here.
    let cfg = config(&bench, Some("operator"), Some("plc_pw"));
    let spec = compile(&cfg, &secret).unwrap();
    assert_eq!(spec.auth, Auth::UserPass { username: "operator".into(), password: "s3cret".into() });
    assert!(!format!("{spec:?}").contains("s3cret"), "the password shows in no debug form");
    // The value is handed through whole: names are trimmed, a password is not.
    let padded = compile(&cfg, &|e: &str| (e == "plc_pw").then(|| " s3cret\r".to_string())).unwrap();
    assert_eq!(padded.auth, Auth::UserPass { username: "operator".into(), password: " s3cret\r".into() });
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = spawn(spec.clone(), tx, WriteGate::new(false)).unwrap();
    let mut seen = Seen { states: Vec::new(), polls: Vec::new() };
    let mut dedup = DedupState::default();
    let mut seq = 0u64;
    assert!(wait(Duration::from_secs(10), || {
        drain(&rx, &spec, &mut dedup, &mut seq, &mut seen);
        !seen.polls.is_empty()
    }));
    assert!(seen.states.contains(&ConnState::Connected));
    worker.handle.join().unwrap();
    // The entry not in the secrets: refused at compile, naming the entry and never a value.
    assert_eq!(
        compile(&cfg, &no_secret).err(),
        Some(OpcError::SecretMissing { source_id: "plc".into(), user: "operator".into(), entry: "plc_pw".into() })
    );
    assert_eq!(
        compile(&cfg, &no_secret).unwrap_err().to_string(),
        "source `plc`: the secret `plc_pw` for the password of `operator` is not in the secrets file"
    );
    // The entry is the only name asked: secrets that hold every other name, the source id and the
    // user among them, still refuse the source by its entry, so no fallback hides a renamed entry.
    let all_but_the_entry = |e: &str| (e != "plc_pw").then(|| "s3cret".to_string());
    assert_eq!(
        compile(&cfg, &all_but_the_entry).err(),
        Some(OpcError::SecretMissing { source_id: "plc".into(), user: "operator".into(), entry: "plc_pw".into() })
    );
    // A wrong password: the worker retries and never connects.
    let wrong = compile(&cfg, &|_| Some("nope".to_string())).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = spawn(wrong.clone(), tx, WriteGate::new(false)).unwrap();
    let mut seen = Seen { states: Vec::new(), polls: Vec::new() };
    assert!(wait(Duration::from_secs(15), || {
        drain(&rx, &wrong, &mut dedup, &mut seq, &mut seen);
        seen.states.iter().any(|s| matches!(s, ConnState::Retrying { .. }))
    }), "{:?}", seen.states);
    assert!(!seen.states.contains(&ConnState::Connected));
    assert!(seen.polls.is_empty());
    worker.handle.join().unwrap();
    bench.stop();
}

#[test]
fn every_configuration_mistake_is_refused_by_name_before_anything_connects() {
    let base = || OpcSourceConfig {
        source_id: " plc ".into(),
        display_name: " Line 1 ".into(),
        endpoint_url: " opc.tcp://127.0.0.1:1/ ".into(),
        user: None,
        secret: None,
        poll_ms: 100,
        rules: vec![OpcRuleConfig { name: "t".into(), store_on_change: false, tags: vec![tag("ns=2;s=a", "a", "")] }],
    };
    let spec = compile(&base(), &no_secret).unwrap();
    assert_eq!((spec.source_id.as_str(), spec.display_name.as_str(), spec.endpoint_url.as_str()), ("plc", "Line 1", "opc.tcp://127.0.0.1:1/"), "trimmed");
    assert_eq!(spec.rules[0].tags[0].ty, SeriesType::Float, "float when no type is given");
    let sid = || "plc".to_string();
    let refused = |mutate: &dyn Fn(&mut OpcSourceConfig)| {
        let mut c = base();
        mutate(&mut c);
        compile(&c, &no_secret).unwrap_err()
    };
    assert_eq!(refused(&|c| c.endpoint_url = "  ".into()), OpcError::EmptyEndpoint { source_id: sid() });
    assert_eq!(refused(&|c| c.poll_ms = 49), OpcError::PollTooShort { source_id: sid(), ms: 49 });
    assert_eq!(compile(&{ let mut c = base(); c.poll_ms = 50; c }, &no_secret).unwrap().poll, Duration::from_millis(50));
    assert_eq!(refused(&|c| c.user = Some(" ".into())), OpcError::EmptyUser { source_id: sid() });
    assert_eq!(refused(&|c| c.secret = Some("X".into())), OpcError::SecretWithoutUser { source_id: sid() });
    assert_eq!(refused(&|c| c.user = Some("u".into())), OpcError::NoSecret { source_id: sid(), user: "u".into() });
    assert_eq!(refused(&|c| { c.user = Some("u".into()); c.secret = Some(" ".into()); }), OpcError::NoSecret { source_id: sid(), user: "u".into() });
    assert_eq!(refused(&|c| c.rules.clear()), OpcError::NoRules { source_id: sid() });
    assert_eq!(refused(&|c| c.rules[0].name = "Bad".into()), OpcError::RuleName { source_id: sid(), name: "Bad".into(), reason: "the name must start with a lowercase letter or `_`".into() });
    assert_eq!(refused(&|c| c.rules[0].name = "a-b".into()), OpcError::RuleName { source_id: sid(), name: "a-b".into(), reason: "the name may use lowercase letters, digits and `_` alone".into() });
    assert_eq!(refused(&|c| c.rules[0].name = "".into()), OpcError::RuleName { source_id: sid(), name: "".into(), reason: "the name is empty".into() });
    assert_eq!(refused(&|c| c.rules[0].name = "x".repeat(65)), OpcError::RuleName { source_id: sid(), name: "x".repeat(65), reason: "the name is longer than 64".into() });
    assert!(compile(&{ let mut c = base(); c.rules[0].name = "x".repeat(64); c }, &no_secret).is_ok());
    assert_eq!(refused(&|c| c.rules.push(c.rules[0].clone())), OpcError::RuleTwice { source_id: sid(), name: "t".into() });
    assert_eq!(refused(&|c| c.rules[0].tags.clear()), OpcError::NoTags { source_id: sid(), rule: "t".into() });
    assert_eq!(
        refused(&|c| c.rules[0].tags[0].node_id = " ".into()),
        OpcError::NodeId { source_id: sid(), rule: "t".into(), source: NodeIdError::Empty }
    );
    assert!(matches!(refused(&|c| c.rules[0].tags[0].node_id = "ns=2;x=1".into()), OpcError::NodeId { source: NodeIdError::Malformed { .. }, .. }));
    assert_eq!(
        refused(&|c| c.rules[0].tags[0].ty = "double".into()),
        OpcError::Type { source_id: sid(), rule: "t".into(), tag: "ns=2;s=a".into(), ty: "double".into() }
    );
    assert_eq!(parse_type(" INT "), Some(SeriesType::Int));
    assert_eq!(parse_type("time"), Some(SeriesType::Time));
    assert_eq!(parse_type("text"), Some(SeriesType::Text));
    assert_eq!(parse_type("bool"), None);
    assert_eq!(
        refused(&|c| c.rules[0].tags[0].series = "".into()),
        OpcError::NoSeriesNoBits { source_id: sid(), rule: "t".into(), tag: "ns=2;s=a".into() }
    );
    let bit = |b: u8, w: u8, s: &str| OpcBitConfig { bit: b, width: w, series: s.into() };
    assert_eq!(
        refused(&|c| { c.rules[0].tags[0].ty = "text".into(); c.rules[0].tags[0].bits = vec![bit(0, 1, "f")]; }),
        OpcError::BitsOnText { source_id: sid(), rule: "t".into(), tag: "ns=2;s=a".into(), ty: SeriesType::Text }
    );
    assert_eq!(
        refused(&|c| c.rules[0].tags[0].series = "A".into()),
        OpcError::ColumnName { source_id: sid(), rule: "t".into(), column: "A".into(), reason: "the name must start with a lowercase letter or `_`".into() }
    );
    assert_eq!(
        refused(&|c| { c.rules[0].tags[0].ty = "int".into(); c.rules[0].tags[0].bits = vec![bit(0, 1, "a")]; }),
        OpcError::ColumnTwice { source_id: sid(), rule: "t".into(), column: "a".into() }
    );
    assert_eq!(
        refused(&|c| c.rules[0].tags.push(tag("ns=2;s=b", "a", ""))),
        OpcError::ColumnTwice { source_id: sid(), rule: "t".into(), column: "a".into() }
    );
    assert_eq!(
        refused(&|c| { c.rules[0].tags[0].ty = "int".into(); c.rules[0].tags[0].bits = vec![bit(0, 1, "f"), bit(1, 1, "f")]; }),
        OpcError::ColumnTwice { source_id: sid(), rule: "t".into(), column: "f".into() }
    );
    assert_eq!(
        refused(&|c| { c.rules[0].tags[0].ty = "int".into(); c.rules[0].tags[0].bits = vec![bit(64, 1, "f")]; }),
        OpcError::BitTooHigh { source_id: sid(), rule: "t".into(), series: "f".into(), bit: 64 }
    );
    assert_eq!(
        refused(&|c| { c.rules[0].tags[0].ty = "int".into(); c.rules[0].tags[0].bits = vec![bit(0, 1, "1f")]; }),
        OpcError::ColumnName { source_id: sid(), rule: "t".into(), column: "1f".into(), reason: "the name must start with a lowercase letter or `_`".into() }
    );
    // A bits-only tag has no column of its own; a width of 0 reads as 1; bit 63 is the last.
    let ok = compile(&{ let mut c = base(); c.rules[0].tags[0].series = "".into(); c.rules[0].tags[0].ty = "int".into(); c.rules[0].tags[0].bits = vec![bit(63, 0, "top")]; c }, &no_secret).unwrap();
    assert_eq!(ok.rules[0].columns(), vec![("top".to_string(), SeriesType::Int)]);
    assert_eq!(ok.rules[0].tags[0].bits[0].width, 1);
    // The exact texts.
    assert_eq!(
        refused(&|c| { c.rules[0].tags[0].ty = "int".into(); c.rules[0].tags[0].bits = vec![bit(64, 1, "f")]; }).to_string(),
        "source `plc`, rule `t`, bit `f`: bit 64 is past the widest word (0 to 63)"
    );
    assert_eq!(refused(&|c| c.poll_ms = 0).to_string(), "source `plc`: the poll interval of 0 ms is under 50 ms");
    assert_eq!(check_name("_ok_1"), Ok(()));
    assert_eq!(check_name("ok one"), Err("the name may use lowercase letters, digits and `_` alone".into()));
    // Every misspelled key refused when read from TOML.
    let text = "source_id = \"plc\"\nendpoint_url = \"opc.tcp://h:1/\"\n[[rules]]\nname = \"t\"\ntags = [ { node_id = \"ns=2;s=a\", series = \"a\" } ]\n";
    let cfg: OpcSourceConfig = toml::from_str(text).unwrap();
    assert_eq!(cfg.poll_ms, 1000, "the default poll");
    assert!(toml::from_str::<OpcSourceConfig>(&text.replace("endpoint_url", "endpoint")).is_err());
    assert!(toml::from_str::<OpcSourceConfig>(&text.replace("tags = [ { node_id", "tags = [ { nodeid")).is_err());
    assert!(toml::from_str::<OpcSourceConfig>(&text.replace("name = \"t\"", "name = \"t\"\nstore_on_changes = true")).is_err());
    let with_bits = text.replace("series = \"a\" }", "series = \"a\", type = \"int\", bits = [ { bit = 1, series = \"f\" } ] }");
    let cfg: OpcSourceConfig = toml::from_str(&with_bits).unwrap();
    assert_eq!(cfg.rules[0].tags[0].bits[0].width, 1, "the default width");
    assert!(toml::from_str::<OpcSourceConfig>(&with_bits.replace("bit = 1", "bits = 1")).is_err());
    // Bits on a float tag: core never carves a float's wire word, so compile refuses it as it
    // refuses text and time, by the same text.
    assert_eq!(
        refused(&|c| {
            c.rules[0].tags[0].ty = "float".into();
            c.rules[0].tags[0].bits = vec![bit(0, 1, "f")];
        }),
        OpcError::BitsOnText { source_id: sid(), rule: "t".into(), tag: "ns=2;s=a".into(), ty: SeriesType::Float }
    );
    assert_eq!(
        refused(&|c| {
            c.rules[0].tags[0].ty = "float".into();
            c.rules[0].tags[0].bits = vec![bit(0, 1, "f")];
        })
        .to_string(),
        "source `plc`, rule `t`, tag `ns=2;s=a`: bits are carved out of an int word, not a float tag"
    );
    // A misspelled optional key is refused as an unknown field in every table (a misspelled
    // required key would be refused for the missing one even without the check).
    let unknown = |t: &str| toml::from_str::<OpcSourceConfig>(t).unwrap_err().to_string();
    assert!(unknown(&text.replace("endpoint_url =", "poll = 200\nendpoint_url =")).contains("unknown field `poll`"));
    assert!(unknown(&text.replace("series = \"a\" }", "series = \"a\", writeable = true }")).contains("unknown field `writeable`"));
    assert!(unknown(&text.replace("name = \"t\"", "name = \"t\"\nstore_on_changes = true")).contains("unknown field `store_on_changes`"));
    assert!(unknown(&with_bits.replace("bit = 1", "bit = 1, widht = 3")).contains("unknown field `widht`"));
    assert!(unknown(&text.replace("endpoint_url", "endpoint")).contains("unknown field `endpoint`"));
    assert!(unknown(&text.replace("tags = [ { node_id", "tags = [ { nodeid")).contains("unknown field `nodeid`"));
    assert!(unknown(&with_bits.replace("bit = 1", "bits = 1")).contains("unknown field `bits`"));
}
