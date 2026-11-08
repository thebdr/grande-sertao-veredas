//! A portal telegram channel (contract: the channel clause): configured by a channel file, a
//! catalog and reply rules, checked before it binds, reading every message into a typed row and
//! answering by rule only while the master switch is on; against the bench's machine, its
//! simulated controller and its pressure, over real loopback sockets, started from a bench file.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use veredas_core::opc::worker::WriteGate;
use veredas_core::telegram::{Role, catalog, schema};
use veredas_maquinar::bench::{self, Bench};
use veredas_portal::channel::{
    ChannelConfig, ChannelError, ChannelEvent, ChannelHandle, LinkState, Literal, MAX_REPEAT_COLUMNS, MessageTable, ReplyField,
    ReplyOutcome, ReplyRule, SendError, spawn,
};
use veredas_portal::{SeriesType, Value};

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

fn channel_text() -> String {
    blocks("channel-schema.md")[0].clone()
}

/// The controller reference's catalog: six messages.
fn catalog_text() -> String {
    blocks("controller-schema.md")[0].clone()
}

fn machine_text() -> String {
    blocks("controller-schema.md")[1].clone()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
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

/// The machine of the controller reference, alone in a bench file, listening (or dialling) on
/// `port`: the plant the gateway's channel meets.
fn machine_bench(dir: &Path, role: &str, port: u16, catalog: &str) -> Bench {
    std::fs::write(dir.join("channel.toml"), channel_text()).unwrap();
    std::fs::write(dir.join("catalog.toml"), catalog).unwrap();
    std::fs::write(dir.join("machine.toml"), machine_text()).unwrap();
    let text = format!(
        "format = \"veredas-bench\"\nversion = 1\nseed = 7\n\n[[channel]]\nname = \"machine\"\nrole = \"{role}\"\nport = {port}\nchannel = \"channel.toml\"\ncatalog = \"catalog.toml\"\ncontroller = \"machine.toml\"\n"
    );
    std::fs::write(dir.join("bench.toml"), &text).unwrap();
    Bench::start(bench::load(&text, dir).unwrap()).unwrap()
}

fn field(name: &str, from: Option<&str>, value: Option<Literal>) -> ReplyField {
    ReplyField { name: name.into(), from: from.map(String::from), value }
}

/// The reply every announcement gets: the item back, destination 1, flag 1 (delivered).
fn answer_rule() -> ReplyRule {
    ReplyRule {
        on: "announce".into(),
        send: "answer".into(),
        fields: vec![
            field("item", Some("item"), None),
            field("dest", None, Some(Literal::Int(1))),
            field("flag", None, Some(Literal::Int(1))),
        ],
    }
}

fn config(role: Role, replies: Vec<ReplyRule>) -> ChannelConfig {
    ChannelConfig {
        source_id: "sorter".into(),
        display_name: "the sorter".into(),
        role,
        spec: schema::load(&channel_text()).unwrap(),
        catalog: catalog::load(&catalog_text()).unwrap(),
        replies,
    }
}

fn dial(port: u16) -> Role {
    Role::Connect { host: "127.0.0.1".into(), port }
}

/// A message as received: its name, its fields, the raw frame, the index clock, the arrival clock.
type Received = (String, HashMap<String, String>, Vec<u8>, i64, i64);

/// Drain events, keeping the messages by name and every reply outcome.
struct Seen {
    messages: Vec<Received>,
    replies: Vec<(String, String, ReplyOutcome)>,
    unreadable: Vec<(Option<u64>, Vec<u8>)>,
    states: Vec<LinkState>,
    keepalives: u64,
}

impl Seen {
    fn new() -> Seen {
        Seen { messages: Vec::new(), replies: Vec::new(), unreadable: Vec::new(), states: Vec::new(), keepalives: 0 }
    }

    fn drain(&mut self, rx: &Receiver<ChannelEvent>) {
        while let Ok(ev) = rx.try_recv() {
            match ev {
                ChannelEvent::Message { name, fields, raw, index_ms, arrival_ms, source_id, id, .. } => {
                    assert_eq!(source_id, "sorter");
                    assert!((21..=26).contains(&id), "{id}");
                    self.messages.push((name, fields, raw, index_ms, arrival_ms));
                }
                ChannelEvent::Reply { on, send, outcome, .. } => self.replies.push((on, send, outcome)),
                ChannelEvent::Unreadable { id, body, .. } => self.unreadable.push((id, body)),
                ChannelEvent::Status { state, .. } => self.states.push(state),
                ChannelEvent::KeepAlive { .. } => self.keepalives += 1,
            }
        }
    }

    fn named(&self, name: &str) -> Vec<&HashMap<String, String>> {
        self.messages.iter().filter(|(n, ..)| n == name).map(|(_, f, ..)| f).collect()
    }
}

fn machine_status(bench: &Bench) -> veredas_maquinar::controller::ControllerStatus {
    bench.channel("machine").unwrap().status().controller.unwrap()
}

// ---- the tables -----------------------------------------------------------------------------

#[test]
fn a_message_s_table_is_typed_by_the_catalog_and_a_row_has_a_hole_where_a_name_is_missing() {
    let cat = catalog::load(&catalog_text()).unwrap();
    let announce = MessageTable::of("c", cat.by_name("announce").unwrap()).unwrap();
    assert_eq!(announce.name, "announce");
    assert_eq!(
        announce.columns,
        vec![
            ("item".to_string(), SeriesType::Int),
            ("lane".to_string(), SeriesType::Int),
            ("read".to_string(), SeriesType::Text),
            ("code".to_string(), SeriesType::Text),
            ("weight".to_string(), SeriesType::Int),
        ]
    );
    let outcome = MessageTable::of("c", cat.by_name("outcome").unwrap()).unwrap();
    assert_eq!(
        outcome.columns,
        vec![("item".to_string(), SeriesType::Text), ("dest".to_string(), SeriesType::Text), ("result".to_string(), SeriesType::Text)],
        "payload groups are text, in pattern order"
    );
    let fields: HashMap<String, String> =
        [("item", "7"), ("lane", "2"), ("read", "Y"), ("code", "PKG7"), ("weight", "412")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    assert_eq!(
        announce.row(&fields),
        vec![Some(Value::Int(7)), Some(Value::Int(2)), Some(Value::Text("Y".into())), Some(Value::Text("PKG7".into())), Some(Value::Int(412))]
    );
    let mut partial = fields.clone();
    partial.remove("lane");
    partial.insert("weight".into(), "heavy".into());
    let row = announce.row(&partial);
    assert_eq!(row[1], None, "a missing name is a hole");
    assert_eq!(row[4], None, "a number that is not one is a hole");

    // A repeat adds its numbered names, up to the cap.
    let with_repeat = "format = \"veredas-catalog\"\nversion = 1\nid_field = \"id\"\n\n[[message]]\nname = \"list\"\nid = 1\nrepeat = [ { name = \"code\", pattern = \"(\\\\w+)\", max = 3 } ]\npayload = \"^(?P<head>\\\\w+):\"\n".to_string();
    let cat = catalog::load(&with_repeat).unwrap();
    let list = MessageTable::of("c", cat.by_name("list").unwrap()).unwrap();
    assert_eq!(
        list.columns.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        ["head", "code1", "code2", "code3"]
    );
    assert!(list.columns.iter().all(|(_, t)| *t == SeriesType::Text));
    let fields: HashMap<String, String> = [("head", "A"), ("code1", "x")].iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    assert_eq!(list.row(&fields), vec![Some(Value::Text("A".into())), Some(Value::Text("x".into())), None, None]);
    let too_wide = with_repeat.replace("max = 3", &format!("max = {}", MAX_REPEAT_COLUMNS + 1));
    let cat = catalog::load(&too_wide).unwrap();
    assert_eq!(
        MessageTable::of("c", cat.by_name("list").unwrap()).err(),
        Some(ChannelError::TooManyColumns { source_id: "c".into(), message: "list".into(), repeat: "code".into(), max: MAX_REPEAT_COLUMNS + 1 })
    );
    let at_cap = with_repeat.replace("max = 3", &format!("max = {MAX_REPEAT_COLUMNS}"));
    let cat = catalog::load(&at_cap).unwrap();
    assert_eq!(MessageTable::of("c", cat.by_name("list").unwrap()).unwrap().columns.len(), 1 + MAX_REPEAT_COLUMNS);
}

// ---- against the bench -----------------------------------------------------------------------

#[test]
fn the_machine_s_messages_are_read_into_rows_and_answered_by_rule_while_the_switch_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let bench = machine_bench(dir.path(), "listen", port, &catalog_text());
    let gate = WriteGate::new(true);
    let (tx, rx) = std::sync::mpsc::channel();
    let cfg = config(dial(port), vec![answer_rule()]);
    let framing = cfg.spec.framing.clone();
    let worker = spawn(cfg, tx, gate.clone()).unwrap();
    assert_eq!(worker.writer.source_id, "sorter");
    assert!(worker.writer.is_armed());
    assert_eq!(worker.writer.tables().iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["announce", "answer", "outcome", "command", "heartbeat", "status"]);

    let mut seen = Seen::new();
    assert!(wait(Duration::from_secs(10), || {
        seen.drain(&rx);
        seen.named("announce").len() >= 5 && seen.named("outcome").len() >= 3 && seen.named("heartbeat").len() >= 4
    }), "announce {} outcome {} heartbeat {}", seen.named("announce").len(), seen.named("outcome").len(), seen.named("heartbeat").len());

    // The link's state: waiting, then linked to one peer.
    assert_eq!(seen.states, [LinkState::Waiting, LinkState::Linked { peers: 1 }]);

    // The announcements, as the rules file says the machine renders them: the counter from 1,
    // the lane cycling, the flag, the template, a weight in range.
    let announces = seen.named("announce");
    let table = worker.writer.tables().iter().find(|t| t.name == "announce").unwrap().clone();
    for (i, f) in announces.iter().enumerate() {
        let row = table.row(f);
        assert_eq!(row[0], Some(Value::Int(i as i64 + 1)), "{f:?}");
        assert_eq!(row[1], Some(Value::Int(if i % 2 == 0 { 1 } else { 2 })), "{f:?}");
        // One item in ten carries the no-read fault: its flag `N` and an empty code.
        if row[2] == Some(Value::Text("N".into())) {
            assert_eq!(row[3], Some(Value::Text("".into())), "{f:?}");
        } else {
            assert_eq!(row[2], Some(Value::Text("Y".into())), "{f:?}");
            assert_eq!(row[3], Some(Value::Text(format!("PKG{}", i + 1).into())), "{f:?}");
        }
        match &row[4] {
            Some(Value::Int(w)) => assert!((100..=900).contains(w), "{f:?}"),
            other => panic!("{other:?}"),
        }
    }
    // Every announcement was answered by the rule, and the outcomes come back to destination 1
    // with a result from the mix, as text rows of the outcome table.
    let sent: Vec<&(String, String, ReplyOutcome)> = seen.replies.iter().filter(|(on, send, o)| on == "announce" && send == "answer" && *o == ReplyOutcome::Sent).collect();
    assert!(sent.len() >= announces.len(), "{} replies sent for {} announcements", sent.len(), announces.len());
    assert!(seen.replies.iter().all(|(_, _, o)| *o == ReplyOutcome::Sent), "{:?}", seen.replies);
    let outcome_table = worker.writer.tables().iter().find(|t| t.name == "outcome").unwrap().clone();
    for f in seen.named("outcome") {
        let row = outcome_table.row(f);
        assert_eq!(row[1], Some(Value::Text("1".into())), "{f:?}");
        assert!(matches!(&row[2], Some(Value::Text(r)) if r.as_ref() == "SORTED" || r.as_ref() == "LOST"), "{f:?}");
        let item: i64 = f["item"].parse().unwrap();
        assert!(item >= 1);
    }
    // Heartbeats: one per destination with its word, as ints.
    let hb_table = worker.writer.tables().iter().find(|t| t.name == "heartbeat").unwrap().clone();
    let dests: std::collections::BTreeSet<i64> = seen.named("heartbeat").iter().filter_map(|f| match hb_table.row(f)[0] { Some(Value::Int(d)) => Some(d), _ => None }).collect();
    assert_eq!(dests.into_iter().collect::<Vec<_>>(), [1, 2, 3, 4]);
    let full = seen.named("heartbeat").into_iter().find(|f| f["dest"] == "4").unwrap();
    assert_eq!(hb_table.row(full)[1], Some(Value::Int(1)), "destination 4 starts full: its bit set");
    // Every message carries the two clocks and its raw frame, which decodes again to the same id.
    let now = veredas_portal::now_unix_ms();
    for (name, _, raw, index_ms, arrival_ms) in &seen.messages {
        assert!(*arrival_ms > now - 60_000 && *arrival_ms <= now + 1, "{name}: {arrival_ms}");
        let offset = *index_ms - *arrival_ms;
        let expected = chrono::Local::now().offset().local_minus_utc() as i64 * 1000;
        assert!((offset - expected).abs() < 5_000, "{name}: the index is the plant-local clock ({offset} vs {expected})");
        let (frame, used) = framing.decode(raw).unwrap();
        assert_eq!(used, raw.len());
        assert!(!frame.segments.is_empty());
    }
    // The machine's own count agrees: what it announced, the gateway answered.
    let status = machine_status(&bench);
    assert!(status.stats.answered >= 5 && status.stats.answered <= status.stats.announced, "{:?}", status.stats);
    assert_eq!(status.stats.unknown_answers, 0);
    assert_eq!(status.stats.invalid_destinations, 0);
    let portal = worker.writer.status();
    assert_eq!(portal.replies_sent, sent.len() as u64);
    assert_eq!((portal.replies_withheld, portal.replies_refused, portal.unknown_ids, portal.undecodable), (0, 0, 0, 0));
    assert_eq!(portal.sent.get("answer").copied(), Some(sent.len() as u64));
    assert!(portal.received.get("announce").copied().unwrap() >= 5);
    assert_eq!(portal.peers.len(), 1);
    assert_eq!(portal.last_error, None);

    // A prompt stop, and the machine sees its peer go.
    let started = Instant::now();
    worker.handle.join().unwrap();
    assert!(started.elapsed() < Duration::from_millis(1500), "{:?}", started.elapsed());
    seen.drain(&rx);
    assert_eq!(seen.states.last(), Some(&LinkState::Stopped));
    assert_eq!(worker.writer.status().wire.state, "stopped");
    assert!(wait(Duration::from_secs(5), || bench.channel("machine").unwrap().status().peers.is_empty()));
    bench.stop();
}

#[test]
fn with_the_switch_off_nothing_is_answered_and_arming_it_live_answers_the_next_item() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let bench = machine_bench(dir.path(), "listen", port, &catalog_text());
    let gate = WriteGate::new(false);
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = spawn(config(dial(port), vec![answer_rule()]), tx, gate.clone()).unwrap();
    assert!(!worker.writer.is_armed());
    let mut seen = Seen::new();
    assert!(wait(Duration::from_secs(10), || {
        seen.drain(&rx);
        seen.named("announce").len() >= 5
    }));
    // Read and recorded, never answered: the plant waits on every item.
    assert!(seen.replies.iter().all(|(_, _, o)| *o == ReplyOutcome::Withheld), "{:?}", seen.replies);
    assert!(seen.replies.len() >= 5);
    assert!(seen.named("outcome").is_empty(), "no answer, no outcome");
    let status = machine_status(&bench);
    assert_eq!(status.stats.answered, 0, "{:?}", status.stats);
    assert!(status.awaiting.len() >= 5);
    let portal = worker.writer.status();
    assert_eq!(portal.replies_sent, 0);
    assert!(portal.replies_withheld >= 5);
    assert_eq!(portal.sent.get("answer"), None);
    assert_eq!(portal.wire.sent, 1, "the connect keep-alive is all that went out");

    // The switch is live: armed, the next announcement is answered.
    gate.set(true);
    assert!(worker.writer.is_armed());
    assert!(wait(Duration::from_secs(5), || machine_status(&bench).stats.answered >= 1));
    assert!(wait(Duration::from_secs(5), || {
        seen.drain(&rx);
        seen.replies.iter().any(|(_, _, o)| *o == ReplyOutcome::Sent) && !seen.named("outcome").is_empty()
    }));
    // Disarmed again: withheld again.
    gate.set(false);
    let withheld_before = worker.writer.status().replies_withheld;
    assert!(wait(Duration::from_secs(5), || worker.writer.status().replies_withheld > withheld_before));
    let answered = machine_status(&bench).stats.answered;
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(machine_status(&bench).stats.answered, answered, "nothing answered while off");
    worker.handle.join().unwrap();
    bench.stop();
}

#[test]
fn the_handle_s_send_is_gated_and_reaches_the_plant_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let bench = machine_bench(dir.path(), "listen", port, &catalog_text());
    let gate = WriteGate::new(false);
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = spawn(config(dial(port), vec![]), tx, gate.clone()).unwrap();
    let writer: ChannelHandle = worker.writer.clone();
    let mut seen = Seen::new();
    assert!(wait(Duration::from_secs(10), || {
        seen.drain(&rx);
        !seen.named("announce").is_empty()
    }));
    let stop = HashMap::from([("code".to_string(), "0".to_string())]);
    // Disarmed: refused before anything is composed; the machine runs on.
    assert_eq!(writer.send(None, "command", stop.clone()), Err(SendError::Disarmed));
    assert_eq!(machine_status(&bench).registers["running"], 1);
    assert_eq!(machine_status(&bench).stats.commands, 0);
    // Armed: an unknown message, a missing value and a value that does not fit are refused by
    // the catalog, and nothing reaches the plant.
    gate.set(true);
    assert_eq!(writer.send(None, "nope", HashMap::new()), Err(SendError::UnknownMessage("nope".into())));
    match writer.send(None, "command", HashMap::new()) {
        Err(SendError::Compose { message, reason }) => {
            assert_eq!(message, "command");
            assert!(reason.contains("code"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    match writer.send(None, "command", HashMap::from([("code".to_string(), "300".to_string())])) {
        Err(SendError::Compose { message, reason }) => {
            assert_eq!(message, "command");
            assert!(reason.contains("300"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(writer.send(Some(999), "command", stop.clone()), Err(SendError::NoPeer(999)));
    assert_eq!(machine_status(&bench).stats.commands, 0);
    assert_eq!(writer.status().sent.get("command"), None);
    // A command the catalog writes: the machine obeys it, the register reads 0, the
    // announcements stop.
    assert_eq!(writer.send(None, "command", stop.clone()), Ok(()));
    assert!(wait(Duration::from_secs(5), || machine_status(&bench).registers["running"] == 0));
    assert_eq!(machine_status(&bench).stats.commands, 1);
    assert_eq!(writer.status().sent.get("command"), Some(&1));
    // An item announced in the pass that took the command may still be in flight: let it land.
    std::thread::sleep(Duration::from_millis(300));
    seen.drain(&rx);
    let announced = seen.named("announce").len();
    std::thread::sleep(Duration::from_millis(500));
    seen.drain(&rx);
    assert_eq!(seen.named("announce").len(), announced, "a stopped machine announces nothing");
    // To one peer, by id: the machine's peer id as the status shows it.
    let peer = writer.status().peers[0];
    assert_eq!(writer.send(Some(peer), "command", HashMap::from([("code".to_string(), "7".to_string())])), Ok(()));
    assert!(wait(Duration::from_secs(5), || machine_status(&bench).registers["speed"] == 70));
    assert_eq!(writer.status().last_error.as_deref(), Some("peer 999 is not connected"), "the last refusal stays named");
    worker.handle.join().unwrap();
    assert_eq!(writer.send(None, "command", stop), Err(SendError::WorkerGone));
    bench.stop();
}

#[test]
fn pressure_turned_on_the_bench_shows_in_the_rows() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let bench = machine_bench(dir.path(), "listen", port, &catalog_text());
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = spawn(config(dial(port), vec![answer_rule()]), tx, WriteGate::new(true)).unwrap();
    let mut seen = Seen::new();
    assert!(wait(Duration::from_secs(10), || {
        seen.drain(&rx);
        seen.named("announce").len() >= 3
    }));
    // Every item a no-read: the flag and the empty code, as the rules file sets them.
    bench.channel("machine").unwrap().set_fault("noread", 100).unwrap();
    let before = seen.named("announce").len();
    assert!(wait(Duration::from_secs(10), || {
        seen.drain(&rx);
        seen.named("announce").len() >= before + 5
    }));
    let after: Vec<&HashMap<String, String>> = seen.named("announce").into_iter().skip(before + 1).collect();
    assert!(after.len() >= 4);
    for f in &after {
        assert_eq!((f["read"].as_str(), f["code"].as_str()), ("N", ""), "{f:?}");
    }
    // A faster rate: more items per second.
    bench.channel("machine").unwrap().set_fault("noread", 0).unwrap();
    bench.channel("machine").unwrap().set_interval(20).unwrap();
    seen.drain(&rx);
    let before = seen.named("announce").len();
    std::thread::sleep(Duration::from_secs(1));
    seen.drain(&rx);
    let per_second = seen.named("announce").len() - before;
    assert!(per_second >= 30, "{per_second} items in a second at 20 ms");
    worker.handle.join().unwrap();
    bench.stop();
}

#[test]
fn an_unknown_id_and_a_refused_body_are_counted_and_shown() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    // The plant's catalog has a message the gateway's lacks (id 99) and a command one byte
    // longer than the gateway's.
    let plant_catalog = catalog_text().replace("name = \"command\"\nid = 24\nlen = 2", "name = \"command\"\nid = 24\nlen = 3")
        + "\n[[message]]\nname = \"extra\"\nid = 99\nlen = 2\nfields = [ { name = \"x\", at = 0, ty = \"U16\" } ]\n";
    assert_ne!(plant_catalog, catalog_text());
    std::fs::write(dir.path().join("channel.toml"), channel_text()).unwrap();
    std::fs::write(dir.path().join("catalog.toml"), &plant_catalog).unwrap();
    let text = format!(
        "format = \"veredas-bench\"\nversion = 1\n\n[[channel]]\nname = \"plant\"\nrole = \"listen\"\nport = {port}\nchannel = \"channel.toml\"\ncatalog = \"catalog.toml\"\nmessages = [ {{ message = \"extra\", fields = [ {{ name = \"x\", value = 5 }} ] }}, {{ message = \"command\", fields = [ {{ name = \"code\", value = 1 }} ] }}, {{ message = \"status\", fields = [ {{ name = \"running\", value = 1 }}, {{ name = \"speed\", value = 9 }} ] }} ]\n"
    );
    let bench = Bench::start(bench::load(&text, dir.path()).unwrap()).unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = spawn(config(dial(port), vec![]), tx, WriteGate::new(true)).unwrap();
    let plant = bench.channel("plant").unwrap();
    assert!(wait(Duration::from_secs(5), || !plant.status().peers.is_empty()));
    plant.send_now("extra", HashMap::from([("x".to_string(), "5".to_string())]), None);
    plant.send_now("command", HashMap::from([("code".to_string(), "1".to_string())]), None);
    plant.send_now("status", HashMap::from([("running".to_string(), "1".to_string()), ("speed".to_string(), "9".to_string())]), None);
    let mut seen = Seen::new();
    assert!(wait(Duration::from_secs(5), || {
        seen.drain(&rx);
        seen.unreadable.len() >= 2 && !seen.named("status").is_empty()
    }), "{:?} {:?}", seen.unreadable, seen.messages.len());
    assert_eq!(seen.unreadable.len(), 2);
    let unknown = seen.unreadable.iter().find(|(id, _)| *id == Some(99)).expect("the unknown id");
    assert_eq!(unknown.1, vec![5, 0]);
    let refused = seen.unreadable.iter().find(|(id, _)| *id == Some(24)).expect("the refused body");
    assert_eq!(refused.1, vec![1, 0, 0], "a body of another length than the fixed one");
    assert_eq!(seen.named("status")[0]["speed"], "9");
    let status = worker.writer.status();
    assert_eq!((status.unknown_ids, status.undecodable), (1, 1));
    assert_eq!(status.received.get("status"), Some(&1));
    assert_eq!(status.wire.received, 3);
    worker.handle.join().unwrap();
    bench.stop();
}

#[test]
fn a_listener_waits_for_the_plant_to_dial_and_a_peer_gone_reads_as_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = spawn(config(Role::Listen { bind: "127.0.0.1".into(), port }, vec![answer_rule()]), tx, WriteGate::new(true)).unwrap();
    let mut seen = Seen::new();
    std::thread::sleep(Duration::from_millis(300));
    seen.drain(&rx);
    assert_eq!(seen.states, [LinkState::Waiting]);
    assert_eq!(worker.writer.status().wire.state, format!("listening on 127.0.0.1:{port}"));
    let bench = machine_bench(dir.path(), "connect", port, &catalog_text());
    assert!(wait(Duration::from_secs(10), || {
        seen.drain(&rx);
        seen.named("announce").len() >= 3
    }));
    assert_eq!(seen.states, [LinkState::Waiting, LinkState::Linked { peers: 1 }]);
    assert!(wait(Duration::from_secs(5), || machine_status(&bench).stats.answered >= 3));
    bench.stop();
    assert!(wait(Duration::from_secs(5), || {
        seen.drain(&rx);
        seen.states.last() == Some(&LinkState::Waiting)
    }), "{:?}", seen.states);
    assert!(worker.writer.status().peers.is_empty());
    // A second plant takes the same port's listener.
    let dir2 = tempfile::tempdir().unwrap();
    let bench = machine_bench(dir2.path(), "connect", port, &catalog_text());
    assert!(wait(Duration::from_secs(10), || {
        seen.drain(&rx);
        seen.states.last() == Some(&LinkState::Linked { peers: 1 })
    }));
    worker.handle.join().unwrap();
    bench.stop();
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(), "the port is free after the stop");
}

#[test]
fn a_dialler_with_nobody_there_reports_trouble_and_links_when_the_plant_appears() {
    let dir = tempfile::tempdir().unwrap();
    let port = free_port();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = spawn(config(dial(port), vec![]), tx, WriteGate::new(false)).unwrap();
    let mut seen = Seen::new();
    assert!(wait(Duration::from_secs(10), || {
        seen.drain(&rx);
        matches!(seen.states.last(), Some(LinkState::Trouble(_)))
    }), "{:?}", seen.states);
    assert!(matches!(seen.states.last(), Some(LinkState::Trouble(why)) if why.starts_with("connecting: ")), "{:?}", seen.states);
    assert_eq!(seen.states[0], LinkState::Waiting);
    assert!(worker.writer.status().wire.last_error.is_some());
    let bench = machine_bench(dir.path(), "listen", port, &catalog_text());
    assert!(wait(Duration::from_secs(20), || {
        seen.drain(&rx);
        seen.states.last() == Some(&LinkState::Linked { peers: 1 })
    }), "{:?}", seen.states);
    worker.handle.join().unwrap();
    bench.stop();
}

// ---- refusals before anything binds ----------------------------------------------------------

fn refused(cfg: ChannelConfig) -> ChannelError {
    let (tx, _rx) = std::sync::mpsc::channel();
    match spawn(cfg, tx, WriteGate::new(true)) {
        Ok(_) => panic!("the channel started"),
        Err(e) => e,
    }
}

#[test]
fn every_refusal_names_its_place_and_nothing_dials_before_it_all_holds() {
    let port = free_port();
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let sid = || "sorter".to_string();

    // The catalog's id field must ride the framing: the catalog's own reason, prefixed.
    let mut cfg = config(dial(port), vec![]);
    cfg.catalog = catalog::load(&catalog_text().replace("id_field = \"id\"", "id_field = \"nope\"")).unwrap();
    assert_eq!(
        refused(cfg),
        ChannelError::Catalog { source_id: sid(), reason: "the catalog's id_field `nope` is no field of the framing's segment header".into() }
    );
    let mut cfg = config(dial(port), vec![]);
    cfg.catalog = catalog::load(&catalog_text().replace("id_field = \"id\"", "id_field = \"length\"")).unwrap();
    assert!(matches!(refused(cfg), ChannelError::Catalog { reason, .. } if reason.contains("`length`") && reason.contains("role")));

    // The reply rules against the catalog.
    let rule = |on: &str, send: &str, fields: Vec<ReplyField>| ReplyRule { on: on.into(), send: send.into(), fields };
    let cases: Vec<(ReplyRule, ChannelError)> = vec![
        (rule("nope", "answer", vec![]), ChannelError::UnknownMessage { source_id: sid(), index: 0, message: "nope".into() }),
        (rule("announce", "nope", vec![]), ChannelError::UnknownMessage { source_id: sid(), index: 0, message: "nope".into() }),
        (
            rule("announce", "answer", vec![field("item", Some("item"), None), field("dest", None, Some(Literal::Int(1)))]),
            ChannelError::Unwritten { source_id: sid(), index: 0, send: "answer".into(), name: "flag".into() },
        ),
        (
            rule("announce", "answer", vec![field("item", Some("item"), None), field("dest", None, Some(Literal::Int(1))), field("flag", None, Some(Literal::Int(1))), field("extra", None, Some(Literal::Int(1)))]),
            ChannelError::Unknown { source_id: sid(), index: 0, send: "answer".into(), name: "extra".into() },
        ),
        (
            rule("announce", "answer", vec![field("item", Some("item"), None), field("item", None, Some(Literal::Int(1)))]),
            ChannelError::Twice { source_id: sid(), index: 0, name: "item".into() },
        ),
        (
            rule("announce", "answer", vec![field("item", Some("item"), Some(Literal::Int(1)))]),
            ChannelError::OneSource { source_id: sid(), index: 0, name: "item".into() },
        ),
        (rule("announce", "answer", vec![field("item", None, None)]), ChannelError::OneSource { source_id: sid(), index: 0, name: "item".into() }),
        (
            rule("announce", "answer", vec![field("item", Some("barcode"), None)]),
            ChannelError::NotRead { source_id: sid(), index: 0, on: "announce".into(), from: "barcode".into(), name: "item".into() },
        ),
    ];
    for (rule, expected) in cases {
        let got = refused(config(dial(port), vec![answer_rule(), rule.clone()]));
        // The second rule is the one at fault: index 1.
        let expected_at_1 = match expected {
            ChannelError::UnknownMessage { source_id, message, .. } => ChannelError::UnknownMessage { source_id, index: 1, message },
            ChannelError::Unwritten { source_id, send, name, .. } => ChannelError::Unwritten { source_id, index: 1, send, name },
            ChannelError::Unknown { source_id, send, name, .. } => ChannelError::Unknown { source_id, index: 1, send, name },
            ChannelError::Twice { source_id, name, .. } => ChannelError::Twice { source_id, index: 1, name },
            ChannelError::OneSource { source_id, name, .. } => ChannelError::OneSource { source_id, index: 1, name },
            ChannelError::NotRead { source_id, on, from, name, .. } => ChannelError::NotRead { source_id, index: 1, on, from, name },
            other => other,
        };
        assert_eq!(got, expected_at_1, "{rule:?}");
    }
    // A payload group is a name the message reads (asked of the catalog): a rule taking its
    // fields from the outcome's groups is accepted, on a port nobody listens on, and stopped.
    let other = free_port();
    let accepted = spawn(
        config(dial(other), vec![rule("outcome", "answer", vec![field("item", Some("item"), None), field("dest", Some("dest"), None), field("flag", Some("result"), None)])]),
        std::sync::mpsc::channel().0,
        WriteGate::new(true),
    );
    assert!(accepted.is_ok(), "{:?}", accepted.err());
    accepted.unwrap().handle.join().unwrap();
    // The exact text of a refusal.
    assert_eq!(
        refused(config(dial(port), vec![rule("announce", "answer", vec![field("item", Some("barcode"), None)])])).to_string(),
        "channel `sorter`, replies[0]: `announce` does not read a field named `barcode`, so `item` cannot be taken from it"
    );
    // A repeat too wide for a table refuses the channel.
    let mut cfg = config(dial(port), vec![]);
    cfg.catalog = catalog::load(&(catalog_text() + "\n[[message]]\nname = \"list\"\nid = 30\nrepeat = [ { name = \"code\", pattern = \"(\\\\w+)\", max = 1000 } ]\n")).unwrap();
    assert_eq!(
        refused(cfg).to_string(),
        format!("channel `sorter`: the message `list` repeats `code` up to 1000 times, more than the {MAX_REPEAT_COLUMNS} columns a table holds")
    );
    // Nothing dialled the port through all of it.
    std::thread::sleep(Duration::from_millis(300));
    assert!(matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock), "a refused channel dialled the plant");
    let _ = PathBuf::new();
}

// ---- the first refute round ------------------------------------------------------------------

/// A disarmed send is a refusal like any other: the status keeps its text as the last refusal,
/// although the switch stops it before the worker is asked. Nobody connected, no bench.
#[test]
fn a_disarmed_send_is_the_last_refusal() {
    let gate = WriteGate::new(true);
    let (tx, _rx) = std::sync::mpsc::channel();
    let worker = spawn(config(dial(free_port()), vec![]), tx, gate.clone()).unwrap();
    let w = worker.writer.clone();
    let stop = HashMap::from([("code".to_string(), "0".to_string())]);
    assert_eq!(w.send(Some(999), "command", stop.clone()), Err(SendError::NoPeer(999)));
    assert_eq!(w.status().last_error.as_deref(), Some("peer 999 is not connected"));
    gate.set(false);
    assert_eq!(w.send(None, "command", stop), Err(SendError::Disarmed));
    assert_eq!(w.status().last_error.as_deref(), Some("writes are turned off for this project"));
    worker.handle.join().unwrap();
}

/// The peer is asked for last: with nobody connected, a message the catalog lacks and a value
/// missing are the catalog's refusals, and only a message that could go out is refused for want
/// of a peer. The server maps the two to different answers, so the order is the contract's.
#[test]
fn the_peer_is_asked_for_last_so_the_catalog_s_refusals_come_first_with_nobody_there() {
    let gate = WriteGate::new(true);
    let (tx, _rx) = std::sync::mpsc::channel();
    let worker = spawn(config(dial(free_port()), vec![]), tx, gate).unwrap();
    let w = worker.writer.clone();
    assert_eq!(w.send(None, "nope", HashMap::new()), Err(SendError::UnknownMessage("nope".into())));
    assert!(matches!(w.send(None, "command", HashMap::new()), Err(SendError::Compose { .. })), "{:?}", w.status().last_error);
    assert_eq!(w.send(None, "command", HashMap::from([("code".to_string(), "0".to_string())])), Err(SendError::NoPeers));
    assert_eq!(w.send(Some(7), "command", HashMap::from([("code".to_string(), "0".to_string())])), Err(SendError::NoPeer(7)));
    worker.handle.join().unwrap();
}
