//! The bench channel (contract: the bench channel clause): one TCP telegram link in either
//! role over real loopback sockets, framed and held by core's channel file, speaking the
//! messages of a catalog by name, with cyclic and on-demand messages built from fields and a
//! monitor of what came and went. The controller that may ride it is `tests/controller.rs`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use veredas_core::telegram::Role;
use veredas_core::telegram::catalog::{self, Catalog};
use veredas_core::telegram::schema::{ChannelSpec, load as load_spec};
use veredas_maquinar::channel::*;
use veredas_maquinar::controller::FieldSpec;

fn doc_blocks(name: &str) -> Vec<String> {
    let doc = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs").join(name);
    let text = std::fs::read_to_string(doc).unwrap().replace("\r\n", "\n");
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

/// The channel file's worked example: the segmented layout the catalog's ids ride in.
fn spec() -> ChannelSpec {
    load_spec(&doc_blocks("channel-schema.md")[0]).unwrap()
}

/// The controller file's worked catalog: six messages.
fn catalog() -> Catalog {
    catalog::load(&doc_blocks("controller-schema.md")[0]).unwrap()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn fields(text: &str) -> Vec<FieldSpec> {
    #[derive(serde::Deserialize)]
    struct List {
        fields: Vec<FieldSpec>,
    }
    toml::from_str::<List>(&format!("fields = [ {text} ]")).unwrap().fields
}

fn values(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

fn config(name: &str, role: Role, catalog: Catalog, messages: Vec<CyclicMessage>) -> ChannelConfig {
    ChannelConfig { name: name.into(), role, spec: spec(), catalog, messages, controller: None, seed: Some(1) }
}

fn listen(port: u16) -> Role {
    Role::Listen { bind: "127.0.0.1".into(), port }
}

fn dial(port: u16) -> Role {
    Role::Connect { host: "127.0.0.1".into(), port }
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

fn linked(a: &BenchChannel, b: &BenchChannel) {
    assert!(wait(Duration::from_secs(5), || a.status().peers.len() == 1 && b.status().peers.len() == 1), "the link: {:?} / {:?}", a.status().wire, b.status().wire);
}

#[test]
fn a_channel_in_either_role_speaks_the_catalog_and_the_monitor_shows_what_came_and_went() {
    let port = free_port();
    let status_line = CyclicMessage { message: "status".into(), every_ms: 100, fields: fields("{ name = \"running\", counter = { first = 1 } }, { name = \"speed\", value = 50 }") };
    let heartbeat = CyclicMessage { message: "heartbeat".into(), every_ms: 0, fields: fields("{ name = \"dest\", cycle = [1, 2] }, { name = \"state\", value = 0 }") };
    let listener = start(config("plant", listen(port), catalog(), vec![status_line, heartbeat])).unwrap();
    assert!(wait(Duration::from_secs(2), || listener.status().wire.state.starts_with("listening on")), "{:?}", listener.status().wire);
    let dialer = start(config("unit", dial(port), catalog(), Vec::new())).unwrap();
    linked(&listener, &dialer);
    // Each side numbers the peers it sees; the ids are core's, counted across the process.
    let (lpeer, dpeer) = (listener.status().peers[0], dialer.status().peers[0]);
    let l = listener.status();
    assert_eq!((l.wire.state.as_str(), l.wire.peers, l.last_error.as_deref()), ("1 peer(s)", 1, None));
    assert!(dialer.status().wire.state.starts_with("connected to 127.0.0.1:"), "{:?}", dialer.status().wire);
    assert!(listener.status().recent.iter().any(|l| l.direction == Direction::Note && l.preview.starts_with(&format!("peer {lpeer} connected from 127.0.0.1:"))), "{:?}", listener.status().recent);
    // The cyclic message every 100 ms, decoded by name on the other side, its fields rendered
    // in order (the counter counting), its body previewed as text.
    assert!(wait(Duration::from_secs(5), || dialer.status().received.get("status").copied().unwrap_or(0) >= 5), "{:?}", dialer.status());
    let d = dialer.status();
    let lines: Vec<&RxLine> = d.recent.iter().filter(|l| l.direction == Direction::In && l.message == "status").collect();
    assert!(lines.len() >= 5, "{lines:?}");
    let newest = lines[0];
    assert_eq!((newest.peer, newest.fields["speed"].as_str()), (Some(dpeer), "50"));
    assert_eq!(newest.preview, format!("RUN={};SPEED=50", newest.fields["running"]));
    let runs: Vec<u64> = lines.iter().rev().map(|l| l.fields["running"].parse().unwrap()).collect();
    assert!(runs.windows(2).all(|w| w[1] == w[0] + 1), "the counter counts: {runs:?}");
    assert!(newest.at.len() == 12 && newest.at.as_bytes()[2] == b':', "a time of day with milliseconds: {}", newest.at);
    let l = listener.status();
    assert!(l.sent.get("status").copied().unwrap_or(0) >= d.received["status"], "{:?} / {:?}", l.sent, d.received);
    assert!(l.recent.iter().any(|x| x.direction == Direction::Out && x.message == "status" && x.peer.is_none()), "sent to everyone");
    assert!(l.wire.sent >= 5 && d.wire.received >= 5, "core's own counters: {:?} / {:?}", l.wire, d.wire);
    assert!(l.wire.received == 0 && d.wire.sent <= 1, "the dialer sent no message, at most core's keep-alive on connect: {:?} / {:?}", l.wire, d.wire);
    // On demand: a message with `every_ms` 0 never goes on its own; `send_now` builds it from
    // the values given, to one peer or to everyone.
    assert_eq!(l.sent.get("heartbeat"), None);
    dialer.send_now("command", values(&[("code", "3")]), None);
    listener.send_now("heartbeat", values(&[("dest", "2"), ("state", "1")]), Some(lpeer));
    assert!(wait(Duration::from_secs(5), || listener.status().received.get("command") == Some(&1) && dialer.status().received.get("heartbeat") == Some(&1)));
    let got = listener.status();
    let command = got.recent.iter().find(|l| l.direction == Direction::In && l.message == "command").unwrap();
    assert_eq!((command.fields["code"].as_str(), command.preview.as_str(), command.peer), ("3", "03 00", Some(lpeer)));
    let sent = got.recent.iter().find(|l| l.direction == Direction::Out && l.message == "heartbeat").unwrap();
    assert_eq!((sent.fields["dest"].as_str(), sent.fields["state"].as_str(), sent.peer), ("2", "1", Some(lpeer)));
    assert_eq!(dialer.status().recent.iter().find(|l| l.message == "heartbeat").unwrap().preview, "02 00 01 00");
    // What cannot be built is a note and the last error, never a frame: each of the three
    // refusals read on its own.
    assert_eq!(listener.status().last_error, None);
    let last_error = |expected: &str| {
        assert!(wait(Duration::from_secs(2), || listener.status().last_error.as_deref().map(|e| e.starts_with(expected)).unwrap_or(false)), "{:?}", listener.status().last_error);
    };
    listener.send_now("nope", HashMap::new(), None);
    last_error("nope: the catalog has no such message");
    listener.send_now("heartbeat", values(&[("dest", "2")]), None);
    last_error("heartbeat: no value for `state`");
    listener.send_now("heartbeat", values(&[("dest", "x"), ("state", "1")]), None);
    last_error("heartbeat: `dest`: `x` is not a U16");
    let notes: Vec<String> = listener.status().recent.iter().filter(|l| l.direction == Direction::Note && l.preview.contains("not sent")).map(|l| l.preview.clone()).collect();
    assert_eq!(notes.len(), 3, "{notes:?}");
    assert!(notes.contains(&"heartbeat not sent: no value for `state`".to_string()), "{notes:?}");
    assert!(notes.contains(&"nope not sent: the catalog has no such message".to_string()), "{notes:?}");
    assert!(notes.iter().any(|n| n.starts_with("heartbeat not sent: `dest`: `x` is not a U16")), "{notes:?}");
    assert_eq!(listener.status().sent.get("heartbeat"), Some(&1));
    assert_eq!(dialer.status().received.get("heartbeat"), Some(&1));
    // The monitor keeps the newest 100 lines, newest first.
    for i in 0..130 {
        listener.send_now(&format!("nope{i}"), HashMap::new(), None);
    }
    assert!(wait(Duration::from_secs(5), || listener.status().last_error.as_deref() == Some("nope129: the catalog has no such message")));
    let recent = listener.status().recent;
    assert_eq!(recent.len(), 100);
    let nopes: Vec<u32> = recent.iter().filter_map(|l| l.preview.strip_prefix("nope")).map(|p| p[..p.find(' ').unwrap()].parse().unwrap()).collect();
    assert!(nopes.len() >= 90 && nopes.len() <= 100, "the status lines keep going between the notes: {nopes:?}");
    assert_eq!(nopes[0], 129, "newest first");
    assert!(nopes.windows(2).all(|w| w[0] == w[1] + 1), "in order: {nopes:?}");
    assert!(nopes[nopes.len() - 1] >= 30, "the oldest dropped: {nopes:?}");
    // Stopped: the other side sees the peer go; the port is free again.
    listener.stop();
    assert!(wait(Duration::from_secs(5), || dialer.status().peers.is_empty()));
    assert!(dialer.status().recent.iter().any(|l| l.direction == Direction::Note && l.preview.starts_with(&format!("peer {dpeer} disconnected"))), "{:?}", dialer.status().recent);
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
    assert!(!dialer.is_finished());
    drop(dialer);
}

#[test]
fn what_the_catalog_cannot_read_is_counted_and_shown_not_decoded() {
    // The sender's catalog holds a message the receiver's does not (id 99), and defines the
    // command two bytes longer: an unknown id, and a known one whose body the rules refuse.
    let port = free_port();
    let mut other = doc_blocks("controller-schema.md")[0].clone();
    other.push_str("\n[[message]]\nname = \"extra\"\nid = 99\nlen = 1\nfields = [ { name = \"x\", at = 0, ty = \"U8\" } ]\n");
    let other = other.replace("name = \"command\"\nid = 24\nlen = 2\n", "name = \"command\"\nid = 24\nlen = 4\n");
    let sender_catalog = catalog::load(&other).unwrap();
    assert_eq!(sender_catalog.by_name("command").unwrap().compose_spec.len, Some(4));
    let receiver = start(config("receiver", listen(port), catalog(), Vec::new())).unwrap();
    let sender = start(config("sender", dial(port), sender_catalog, Vec::new())).unwrap();
    linked(&receiver, &sender);
    sender.send_now("extra", values(&[("x", "7")]), None);
    sender.send_now("command", values(&[("code", "1")]), None);
    sender.send_now("status", values(&[("running", "1"), ("speed", "2")]), None);
    assert!(wait(Duration::from_secs(5), || {
        let r = receiver.status();
        r.unknown_ids == 1 && r.undecodable == 1 && r.received.get("status") == Some(&1)
    }), "{:?}", receiver.status());
    let r = receiver.status();
    assert_eq!(r.wire.received, 3, "every frame arrived; two were not messages this catalog reads");
    assert!(r.recent.iter().any(|l| l.direction == Direction::In && l.message == "unknown id 99" && l.preview == "07" && l.fields.is_empty()), "{:?}", r.recent);
    assert!(r.recent.iter().any(|l| l.direction == Direction::In && l.message == "command (body refused)" && l.preview == "01 00 00 00"), "{:?}", r.recent);
    assert_eq!(r.received.len(), 1);
    assert_eq!((sender.status().sent.get("extra"), sender.status().sent.get("command")), (Some(&1), Some(&1)));
    receiver.stop();
    sender.stop();
}

/// The worked channel file without segments: the id rides in the envelope.
fn segmentless_spec() -> ChannelSpec {
    let text = doc_blocks("channel-schema.md")[0].replace("  { name = \"count\", size = 2, role = \"count\" },\n", "  { name = \"id\", size = 2 },\n");
    let start = text.find("segment = [").unwrap();
    let end = start + text[start..].find("]\n").unwrap() + 2;
    let text = format!("{}{}", &text[..start], &text[end..]);
    load_spec(&text).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

#[test]
fn in_a_layout_without_segments_the_id_rides_in_the_envelope() {
    let port = free_port();
    let spec = segmentless_spec();
    assert!(spec.framing.segment.is_none());
    let with = |name: &str, role: Role| ChannelConfig { name: name.into(), role, spec: spec.clone(), catalog: catalog(), messages: Vec::new(), controller: None, seed: Some(1) };
    let listener = start(with("plant", listen(port))).unwrap();
    let dialer = start(with("unit", dial(port))).unwrap();
    linked(&listener, &dialer);
    dialer.send_now("command", values(&[("code", "5")]), None);
    listener.send_now("status", values(&[("running", "1"), ("speed", "9")]), None);
    assert!(wait(Duration::from_secs(5), || listener.status().received.get("command") == Some(&1) && dialer.status().received.get("status") == Some(&1)), "{:?} / {:?}", listener.status(), dialer.status());
    let l = listener.status();
    let command = l.recent.iter().find(|x| x.direction == Direction::In && x.message == "command").unwrap();
    assert_eq!((command.fields["code"].as_str(), command.preview.as_str()), ("5", "05 00"));
    let d = dialer.status();
    assert_eq!(d.recent.iter().find(|x| x.direction == Direction::In && x.message == "status").unwrap().preview, "RUN=1;SPEED=9");
    assert_eq!((l.unknown_ids, l.undecodable, d.unknown_ids, d.undecodable), (0, 0, 0, 0));
    // The id must be an envelope field here, and no field the channel fills in itself.
    let text = doc_blocks("controller-schema.md")[0].replace("id_field = \"id\"", "id_field = \"nope\"");
    let err = start(ChannelConfig { name: "x".into(), role: listen(free_port()), spec: spec.clone(), catalog: catalog::load(&text).unwrap(), messages: Vec::new(), controller: None, seed: None }).err().unwrap();
    assert_eq!(err.to_string(), "the catalog's id_field `nope` is no field of the framing's envelope");
    let text = doc_blocks("controller-schema.md")[0].replace("id_field = \"id\"", "id_field = \"sequence\"");
    let err = start(ChannelConfig { name: "x".into(), role: listen(free_port()), spec, catalog: catalog::load(&text).unwrap(), messages: Vec::new(), controller: None, seed: None }).err().unwrap();
    assert_eq!(err.to_string(), "the catalog's id_field `sequence` is the framing's envelope field with the role `sequence`, which the channel fills in itself: an id cannot ride it");
    listener.stop();
    dialer.stop();
}

#[test]
fn a_channel_refuses_a_configuration_it_cannot_speak_and_reports_a_port_it_cannot_take() {
    // The catalog's id must ride in the framing, in a field the channel does not fill itself.
    let text = doc_blocks("controller-schema.md")[0].replace("id_field = \"id\"", "id_field = \"nope\"");
    let err = start(config("x", listen(free_port()), catalog::load(&text).unwrap(), Vec::new())).err().unwrap();
    assert!(matches!(err, ChannelError::IdField(_)));
    assert_eq!(err.to_string(), "the catalog's id_field `nope` is no field of the framing's segment header");
    let text = doc_blocks("controller-schema.md")[0].replace("id_field = \"id\"", "id_field = \"length\"");
    let err = start(config("x", listen(free_port()), catalog::load(&text).unwrap(), Vec::new())).err().unwrap();
    assert_eq!(err.to_string(), "the catalog's id_field `length` is the framing's segment header field with the role `length`, which the channel fills in itself: an id cannot ride it");
    // Every cyclic message is the catalog's, with every name written, from sources a plain
    // channel has.
    let bad = |message: &str, spec: &str| start(config("x", listen(free_port()), catalog(), vec![CyclicMessage { message: message.into(), every_ms: 0, fields: fields(spec) }])).err().unwrap().to_string();
    assert_eq!(bad("nope", ""), "messages[0]: the catalog has no message `nope`");
    assert_eq!(bad("status", "{ name = \"running\", value = 1 }"), "messages[0] (`status`): messages[0]: the message needs `speed`, which no field writes");
    assert_eq!(bad("status", "{ name = \"running\", from = \"x\" }, { name = \"speed\", value = 1 }"), "messages[0] (`status`): messages[0].fields[0] (`running`): `from` needs a message or an item to read from");
    assert_eq!(bad("status", "{ name = \"running\", register = \"x\" }, { name = \"speed\", value = 1 }"), "messages[0] (`status`): messages[0].fields[0] (`running`): a plain channel has no registers");
    // A port already taken: the channel starts, and says so in its status.
    let squatter = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = squatter.local_addr().unwrap().port();
    let taken = start(config("taken", listen(port), catalog(), Vec::new())).unwrap();
    assert!(wait(Duration::from_secs(5), || taken.status().wire.state == "failed"), "{:?}", taken.status().wire);
    let t = taken.status();
    assert!(t.last_error.as_deref().unwrap_or("").contains(&port.to_string()), "{:?}", t.last_error);
    assert!(t.recent.iter().any(|l| l.direction == Direction::Note && l.preview.contains(&port.to_string())));
    assert!(!taken.is_finished(), "the channel stays, its status saying why it serves nobody, until stopped");
    taken.stop();
    drop(squatter);
    // A dialer with nobody to dial keeps trying, and says so.
    let lonely = start(config("lonely", dial(free_port()), catalog(), Vec::new())).unwrap();
    assert!(wait(Duration::from_secs(5), || lonely.status().wire.state == "retrying"), "{:?}", lonely.status().wire);
    assert!(lonely.status().peers.is_empty() && !lonely.is_finished());
    assert_eq!(lonely.set_interval(1).unwrap_err(), "this channel has no controller");
    lonely.stop();
}
