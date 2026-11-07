//! The simulated controller (contract: the controller clause): the engine of
//! `docs/controller-schema.md`'s worked example driven tick by tick, with no socket and no
//! sleep, every output pinned; then the machine and the unit of that example talking to each
//! other over a real loopback socket through two bench channels, with the bench's knobs turned
//! live.

use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use veredas_core::telegram::Role;
use veredas_core::telegram::catalog::{self, Catalog};
use veredas_core::telegram::schema::{ChannelSpec, load as load_spec};
use veredas_maquinar::channel::{BenchChannel, ChannelConfig, Direction, start};
use veredas_maquinar::controller::*;

fn doc(name: &str) -> String {
    let doc = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs").join(name);
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
    doc("controller-schema.md").split_whitespace().collect::<Vec<_>>().join(" ")
}

fn catalog() -> Catalog {
    catalog::load(&blocks("controller-schema.md")[0]).unwrap()
}

fn machine_rules() -> Rules {
    load(&blocks("controller-schema.md")[1], &catalog()).unwrap()
}

fn unit_rules() -> Rules {
    load(&blocks("controller-schema.md")[2], &catalog()).unwrap()
}

/// The channel file's worked example: the segmented layout the catalog's ids ride in.
fn spec() -> ChannelSpec {
    load_spec(&blocks("channel-schema.md")[0]).unwrap()
}

fn rules(body: &str) -> Rules {
    load(&format!("format = \"veredas-controller\"\nversion = 1\n{body}"), &catalog()).unwrap_or_else(|e| panic!("{e}\n{body}"))
}

fn values(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

/// Tick `ms` forward in steps of 10 ms, collecting everything the controller produced.
fn run(c: &mut Controller, ms: u64, peers: usize) -> Vec<Outgoing> {
    let mut out = Vec::new();
    let mut left = ms;
    while left > 0 {
        let step = left.min(10);
        out.extend(c.tick(step, peers));
        left -= step;
    }
    out
}

/// The messages among what was produced (a note has no message).
fn sent(out: &[Outgoing]) -> Vec<&Outgoing> {
    out.iter().filter(|o| !o.message.is_empty()).collect()
}

/// The announcements among what was produced (the cyclic messages go out beside them).
fn announces(out: &[Outgoing]) -> Vec<&Outgoing> {
    out.iter().filter(|o| o.message == "announce").collect()
}

fn notes(out: &[Outgoing]) -> Vec<String> {
    out.iter().filter(|o| o.message.is_empty()).map(|o| o.note.clone()).collect()
}

fn field<'a>(o: &'a Outgoing, name: &str) -> &'a str {
    o.fields.get(name).map(String::as_str).unwrap_or_else(|| panic!("no field `{name}` in {o:?}"))
}

#[test]
fn the_machine_announces_awaits_the_answer_and_reports_the_outcome_tick_by_tick() {
    let prose = doc_prose();
    let mut m = Controller::new(machine_rules(), None);
    m.set_fault("noread", 0).unwrap();
    // Nothing is announced into nothing: no peer, no item, however long.
    assert!(prose.contains("An item is announced only while a peer is connected (announced into nothing it would only time out) and the gate register, if any, is not 0, a requeued item coming round again included; each is due an interval after the last was due, and a pass of the engine announces every item due since the last pass, at most twenty (a longer stall skips the rest, keeping the phase); when announcing resumes, the first item comes at once."));
    let out = run(&mut m, 1000, 0);
    assert!(announces(&out).is_empty());
    assert_eq!(sent(&out).len(), 4, "the heartbeats go out, peers or none");
    assert_eq!(m.status().stats.announced, 0);
    // A peer: the first item at once, then one every 100 ms, each due an interval after the
    // last was due.
    let out = m.tick(0, 1);
    assert_eq!(out.len(), 1);
    let first = &out[0];
    assert_eq!((first.message.as_str(), first.to, first.note.as_str()), ("announce", None, "announce 1"));
    assert_eq!((field(first, "item"), field(first, "lane"), field(first, "read"), field(first, "code")), ("1", "1", "Y", "PKG1"));
    let weight: u64 = field(first, "weight").parse().unwrap();
    assert!((100..=900).contains(&weight), "{weight}");
    let out = run(&mut m, 1000, 1);
    let announced = announces(&out);
    assert_eq!(announced.len(), 10, "{:?}", notes(&out));
    assert_eq!(announced.iter().map(|o| field(o, "item")).collect::<Vec<_>>(), ["2", "3", "4", "5", "6", "7", "8", "9", "10", "11"]);
    assert_eq!(announced.iter().map(|o| field(o, "lane")).collect::<Vec<_>>(), ["2", "1", "2", "1", "2", "1", "2", "1", "2", "1"], "the lane cycles");
    assert_eq!(field(announced[9], "code"), "PKG11", "the template reads the earlier field");
    let s = m.status();
    assert_eq!(s.stats.announced, 11);
    assert_eq!(s.awaiting.len(), 11);
    assert_eq!(s.awaiting[0].key, "1");
    assert_eq!(s.oldest_unanswered_ms, Some(1000), "item 1 was announced at 1000 and it is 2000 now");
    assert_eq!(s.interval_ms, Some(100));
    assert_eq!(s.faults, BTreeMap::from([("noread".to_string(), 0)]));

    // The answer for item 1 from peer 5, delivered to destination 2: the item travels 500 ms,
    // then the outcome goes to the peer that answered, with the item's and the reply's fields.
    let out = m.on_message("answer", &values(&[("item", " 1 "), ("dest", "2"), ("flag", "1")]), Some(5));
    assert_eq!(notes(&out), ["1 to 2 (1000 ms)"], "the key trimmed; the latency since the announcement");
    assert!(sent(&out).is_empty());
    let s = m.status();
    assert_eq!((s.stats.answered, s.awaiting.len()), (1, 10));
    let out = run(&mut m, 490, 1);
    assert!(sent(&out).iter().all(|o| o.message == "announce"), "nothing reported before the travel ends");
    let out = run(&mut m, 20, 1);
    let outcome = out.iter().find(|o| o.message == "outcome").expect("the outcome 500 ms after the answer");
    assert_eq!((outcome.to, field(outcome, "item"), field(outcome, "dest")), (Some(5), "1", "2"));
    let result = field(outcome, "result");
    assert!(result == "SORTED" || result == "LOST", "drawn from the mix: {result}");
    assert_eq!(outcome.note, format!("outcome for 1 at 2: {}", if result == "SORTED" { "sorted" } else { "lost" }));
    let s = m.status();
    assert_eq!(s.history.len(), 1);
    assert_eq!((s.history[0].key.as_str(), s.history[0].destination, s.history[0].peer, s.history[0].laps), ("1", Some(2), Some(5), 1));
    assert_eq!(s.history[0].ended.as_deref(), Some(if result == "SORTED" { "sorted" } else { "lost" }));
    assert_eq!(s.history[0].reply_latency_ms(), Some(1000));
    assert_eq!(s.stats.ended.values().sum::<u64>(), 1);

    // An answer for an item not awaited is counted and dropped.
    let out = m.on_message("answer", &values(&[("item", "999"), ("dest", "2"), ("flag", "1")]), Some(5));
    assert_eq!(notes(&out), ["answer for 999, which is not awaited"]);
    assert_eq!(m.status().stats.unknown_answers, 1);
    // A destination the machine does not have is counted; the item still travels.
    let out = m.on_message("answer", &values(&[("item", "2"), ("dest", "9"), ("flag", "1")]), Some(5));
    assert_eq!(notes(&out), ["2 to 9, which is unknown (1410 ms)"]);
    assert_eq!(m.status().stats.invalid_destinations, 1);
    let out = run(&mut m, 500, 1);
    assert!(sent(&out).iter().any(|o| o.message == "outcome" && field(o, "item") == "2" && field(o, "dest") == "9"));
    // A message nobody reacts to is nothing.
    assert!(m.on_message("status", &values(&[("running", "1"), ("speed", "1")]), None).is_empty());
    assert_eq!(m.rules().sends(), ["announce", "outcome", "heartbeat", "status"]);
    // A reply that names no destination (no destination field configured) names none: not
    // invalid, the item travels to `-`.
    let mut no_dest = Controller::new(
        rules("[destinations]\nlist = [1]\n[items]\nmessage = \"announce\"\ninterval_ms = 100\nkey = \"item\"\nfields = [ { name = \"item\", counter = {} }, { name = \"lane\", value = 1 }, { name = \"read\", value = \"Y\" }, { name = \"code\", value = \"C\" }, { name = \"weight\", value = 1 } ]\n[items.reply]\nmessage = \"answer\"\n"),
        Some(1),
    );
    no_dest.tick(0, 1);
    let out = no_dest.on_message("answer", &values(&[("item", "1"), ("dest", "9"), ("flag", "1")]), Some(2));
    assert_eq!(notes(&out), ["1 to - (0 ms)"]);
    assert_eq!((no_dest.status().stats.invalid_destinations, no_dest.status().history[0].destination), (0, None));
}

#[test]
fn the_outcome_is_drawn_from_the_mix_by_weight() {
    let prose = doc_prose();
    assert!(prose.contains("drawn from a mix of 9 `sorted` to 1 `lost`"));
    // A thousand items delivered to a chute with no bit set: about nine sorted to one lost.
    let mut m = Controller::new(machine_rules(), Some(2));
    m.set_fault("noread", 0).unwrap();
    m.set_interval(100_000).unwrap();
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for i in 1..=1000 {
        m.announce_now().unwrap();
        m.on_message("answer", &values(&[("item", &i.to_string()), ("dest", "1"), ("flag", "1")]), Some(1));
        for o in m.tick(500, 1) {
            if o.message == "outcome" {
                *counts.entry(field(&o, "result").to_string()).or_default() += 1;
            }
        }
    }
    assert_eq!(counts.values().sum::<u32>(), 1000, "{counts:?}");
    assert!((850..=950).contains(&counts["SORTED"]) && (50..=150).contains(&counts["LOST"]), "{counts:?}");
    assert_eq!((m.status().stats.ended["sorted"], m.status().stats.ended["lost"]), (counts["SORTED"] as u64, counts["LOST"] as u64));
    // A weight of 0 is never drawn, wherever it sits; the shares hold among the rest.
    let three = "[items]\nmessage = \"announce\"\ninterval_ms = 100\nkey = \"item\"\nfields = [ { name = \"item\", counter = {} }, { name = \"lane\", value = 1 }, { name = \"read\", value = \"Y\" }, { name = \"code\", value = \"C\" }, { name = \"weight\", value = 1 } ]\n\
                 [items.reply]\nmessage = \"answer\"\ndestination = \"dest\"\n[items.outcome]\nmessage = \"outcome\"\nfields = [ { name = \"item\", from = \"item\" }, { name = \"dest\", from = \"dest\" }, { name = \"result\", value = \"R\" } ]\n\
                 mix = [ { name = \"never\", weight = 0, set = { result = \"NEVER\" } }, { name = \"a\", weight = 3, set = { result = \"A\" } }, { name = \"b\", weight = 1, set = { result = \"B\" } } ]\n";
    let mut t = Controller::new(rules(three), Some(3));
    t.set_interval(100_000).unwrap();
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for i in 1..=400 {
        t.announce_now().unwrap();
        t.on_message("answer", &values(&[("item", &i.to_string()), ("dest", "1"), ("flag", "1")]), None);
        for o in t.tick(0, 1) {
            if o.message == "outcome" {
                *counts.entry(field(&o, "result").to_string()).or_default() += 1;
            }
        }
    }
    assert_eq!(counts.keys().collect::<Vec<_>>(), ["A", "B"], "{counts:?}");
    assert!((260..=340).contains(&counts["A"]) && (60..=140).contains(&counts["B"]), "{counts:?}");
}

#[test]
fn the_dispositions_requeue_park_or_deliver_and_a_lap_brings_an_item_round_again() {
    let prose = doc_prose();
    assert!(prose.contains("a `flag` of 2 sends the item round again after a lap of 2000 ms, a `flag` of 3 parks it, anything else delivers it"));
    let mut m = Controller::new(machine_rules(), None);
    m.set_fault("noread", 0).unwrap();
    m.set_interval(100_000).unwrap();
    m.tick(0, 1);
    assert_eq!(m.status().awaiting.len(), 1);
    // Round again: the item is announced a second time, 2000 ms later, as lap 2, keeping its
    // fields.
    let out = m.on_message("answer", &values(&[("item", "1"), ("dest", "2"), ("flag", "2")]), Some(5));
    assert_eq!(notes(&out), ["1: no destination yet, round again"]);
    let s = m.status();
    assert_eq!((s.stats.requeued, s.awaiting.len(), s.history.len()), (1, 0, 0));
    assert!(announces(&run(&mut m, 1990, 1)).is_empty());
    assert!(announces(&run(&mut m, 30, 0)).is_empty(), "the lap came round with no peer: the item is held");
    let out = m.tick(0, 1);
    let again = announces(&out);
    assert_eq!(again.len(), 1, "and announced the moment a peer is there");
    assert_eq!((again[0].message.as_str(), again[0].note.as_str(), field(again[0], "item"), field(again[0], "code")), ("announce", "announce 1 (lap 2)", "1", "PKG1"));
    assert_eq!(m.status().awaiting[0].laps, 2);
    assert_eq!(m.status().stats.announced, 2);
    // Parked: ended, asked about no more.
    let out = m.on_message("answer", &values(&[("item", "1"), ("dest", "2"), ("flag", "3")]), Some(5));
    assert_eq!(notes(&out), ["1: parked"]);
    let s = m.status();
    assert_eq!((s.stats.parked, s.awaiting.len(), s.history[0].ended.as_deref()), (1, 0, Some("parked")));
    assert!(announces(&run(&mut m, 3000, 1)).is_empty(), "a parked item does not come round");
    assert_eq!(m.status().stats.ended, BTreeMap::from([("parked".to_string(), 1)]));
    // The first disposition that matches decides; none matching delivers.
    m.announce_now().unwrap();
    let out = m.on_message("answer", &values(&[("item", "2"), ("dest", "3"), ("flag", "7")]), Some(5));
    assert_eq!(notes(&out), ["2 to 3 (0 ms)"]);
    // Without an outcome, a delivered item ends `delivered` at once.
    let mut plain = Controller::new(
        rules("[items]\nmessage = \"announce\"\ninterval_ms = 100\nkey = \"item\"\nfields = [ { name = \"item\", counter = {} }, { name = \"lane\", value = 1 }, { name = \"read\", value = \"Y\" }, { name = \"code\", value = \"C\" }, { name = \"weight\", value = 1 } ]\n[items.reply]\nmessage = \"answer\"\n"),
        Some(1),
    );
    plain.tick(0, 1);
    plain.on_message("answer", &values(&[("item", "1"), ("dest", "3"), ("flag", "1")]), None);
    let s = plain.status();
    assert_eq!((s.history[0].ended.as_deref(), s.history[0].destination, s.stats.ended.get("delivered")), (Some("delivered"), None, Some(&1)));
    // The lap is the file's: 30000 when left out.
    let out = plain.on_message("answer", &values(&[("item", "1"), ("dest", "3"), ("flag", "1")]), None);
    assert_eq!(notes(&out), ["answer for 1, which is not awaited"]);
}

#[test]
fn a_destination_s_state_forces_the_outcome_and_the_bench_s_knobs_set_it() {
    let prose = doc_prose();
    assert!(prose.contains("unless the destination's state forces one: a destination with its `full` bit set forces `FULL` and the item comes round again, one with `fault` set forces `FAULT`. Destinations 1 to 4 rest with no bit set; destination 4 starts `full`."));
    let mut m = Controller::new(machine_rules(), None);
    m.set_fault("noread", 0).unwrap();
    m.set_interval(100_000).unwrap();
    assert_eq!(m.status().destinations, BTreeMap::from([(1, 0), (2, 0), (3, 0), (4, 1)]));
    // Destination 4 is full: the outcome is FULL, forced over the rendered field, and the item
    // comes round again after the lap.
    m.tick(0, 1);
    m.on_message("answer", &values(&[("item", "1"), ("dest", "4"), ("flag", "1")]), Some(2));
    let out = run(&mut m, 500, 1);
    let o = sent(&out);
    assert_eq!(o.len(), 1);
    assert_eq!((o[0].message.as_str(), field(o[0], "result"), field(o[0], "dest"), o[0].note.as_str()), ("outcome", "FULL", "4", "outcome for 1 at 4: blocked:full"));
    let s = m.status();
    assert_eq!((s.stats.requeued, s.stats.ended.get("blocked:full"), s.history[0].ended.as_deref()), (1, Some(&1), Some("blocked:full")));
    let out = run(&mut m, 2000, 1);
    assert_eq!(announces(&out)[0].note, "announce 1 (lap 2)");
    // The bit cleared: the next outcome is drawn from the mix and the item ends.
    m.set_bit(4, "full", false).unwrap();
    m.on_message("answer", &values(&[("item", "1"), ("dest", "4"), ("flag", "1")]), Some(2));
    let out = run(&mut m, 500, 1);
    let result = field(out.iter().find(|o| o.message == "outcome").unwrap(), "result").to_string();
    assert!(result == "SORTED" || result == "LOST", "{result}");
    assert!(m.status().awaiting.is_empty() && m.status().stats.requeued == 1);
    // A fault on a destination: FAULT, and the item ends. The first blocking entry whose bit
    // is set decides: a word with both bits is `full`.
    m.set_destination(3, 2).unwrap();
    m.announce_now().unwrap();
    m.on_message("answer", &values(&[("item", "2"), ("dest", "3"), ("flag", "1")]), Some(2));
    let out = run(&mut m, 500, 1);
    let outcome = |out: &[Outgoing]| out.iter().find(|o| o.message == "outcome").map(|o| field(o, "result").to_string()).unwrap();
    assert_eq!((outcome(&out).as_str(), m.status().stats.requeued), ("FAULT", 1));
    m.set_destination(3, 3).unwrap();
    m.announce_now().unwrap();
    m.on_message("answer", &values(&[("item", "3"), ("dest", "3"), ("flag", "1")]), Some(2));
    let out = run(&mut m, 500, 1);
    assert_eq!(outcome(&out), "FULL");
    assert_eq!(m.status().destinations[&3], 3);
    // Every chute full, then every chute back.
    m.set_bit_everywhere("full", true).unwrap();
    assert_eq!(m.status().destinations, BTreeMap::from([(1, 1), (2, 1), (3, 3), (4, 1)]));
    m.set_bit_everywhere("full", false).unwrap();
    assert_eq!(m.status().destinations, BTreeMap::from([(1, 0), (2, 0), (3, 2), (4, 0)]));
    // The knobs refuse what the rules do not have.
    assert_eq!(m.set_bit(9, "full", true).unwrap_err().to_string(), "no destination 9");
    assert_eq!(m.set_bit(1, "nope", true).unwrap_err().to_string(), "no bit named `nope`");
    assert_eq!(m.set_destination(9, 0).unwrap_err().to_string(), "no destination 9");
    assert_eq!(m.set_fault("nope", 1).unwrap_err().to_string(), "no fault named `nope`");
    assert_eq!(m.set_fault("noread", 101).unwrap_err().to_string(), "101 is over 100");
    assert_eq!(m.set_register("nope", 1).unwrap_err().to_string(), "no register `nope`");
    assert_eq!(m.set_interval(0).unwrap_err().to_string(), "the interval is 0");
    let mut unit = Controller::new(unit_rules(), None);
    assert_eq!(unit.set_interval(5).unwrap_err().to_string(), "no items to announce");
    assert!(unit.announce_now().is_none());
    // An outcome drawn: a mix entry that requeues, and a mix with no weight ending delivered.
    let body = |mix: &str| {
        format!(
            "[items]\nmessage = \"announce\"\ninterval_ms = 100\nkey = \"item\"\nlap_ms = 100\nfields = [ {{ name = \"item\", counter = {{}} }}, {{ name = \"lane\", value = 1 }}, {{ name = \"read\", value = \"Y\" }}, {{ name = \"code\", value = \"C\" }}, {{ name = \"weight\", value = 1 }} ]\n\
             [items.reply]\nmessage = \"answer\"\ndestination = \"dest\"\n[items.outcome]\nmessage = \"outcome\"\nfields = [ {{ name = \"item\", from = \"item\" }}, {{ name = \"dest\", from = \"dest\" }}, {{ name = \"result\", value = \"R\" }} ]\n{mix}"
        )
    };
    let mut again = Controller::new(rules(&body("mix = [ { name = \"back\", weight = 1, set = { result = \"BACK\" }, requeue = true }, { name = \"never\", weight = 0 } ]\n")), Some(1));
    again.tick(0, 1);
    let out = again.on_message("answer", &values(&[("item", "1"), ("dest", "3"), ("flag", "1")]), None);
    let out = [out, again.tick(0, 1)].concat();
    assert_eq!((field(sent(&out)[0], "result"), sent(&out)[0].note.as_str()), ("BACK", "outcome for 1 at 3: back"));
    assert_eq!((again.status().stats.requeued, again.status().stats.ended.get("back")), (1, Some(&1)));
    assert_eq!(sent(&run(&mut again, 100, 1))[0].note, "announce 1 (lap 2)");
    let mut none = Controller::new(rules(&body("mix = [ { name = \"never\", weight = 0 } ]\n")), Some(1));
    none.tick(0, 1);
    none.on_message("answer", &values(&[("item", "1"), ("dest", "3"), ("flag", "1")]), None);
    let out = none.tick(0, 1);
    assert_eq!((field(sent(&out)[0], "result"), sent(&out)[0].note.as_str()), ("R", "outcome for 1 at 3: delivered"));
}

#[test]
fn commands_set_registers_the_gate_stops_the_announcements_and_cyclic_messages_read_them() {
    let prose = doc_prose();
    assert!(prose.contains("The `command` message's `code` sets the registers: 1 starts the machine, 0 stops it, 7 sets `speed` to 70."));
    assert!(prose.contains("A `heartbeat` goes out every 1000 ms for each destination with its state word, and a `status` line every 5000 ms with the registers."));
    let mut m = Controller::new(machine_rules(), None);
    m.set_fault("noread", 0).unwrap();
    // Stopped by a command: no more announcements, however long; the cyclic messages go on.
    let out = m.on_message("command", &values(&[("code", "0")]), Some(1));
    assert_eq!(notes(&out), ["command 0: running = 0"]);
    assert_eq!(m.status().registers["running"], 0);
    let out = run(&mut m, 1000, 1);
    let names: Vec<&str> = sent(&out).iter().map(|o| o.message.as_str()).collect();
    assert_eq!(names, ["heartbeat", "heartbeat", "heartbeat", "heartbeat"], "{:?}", notes(&out));
    let heartbeats = sent(&out);
    assert_eq!(heartbeats.iter().map(|o| (field(o, "dest"), field(o, "state"))).collect::<Vec<_>>(), [("1", "0"), ("2", "0"), ("3", "0"), ("4", "1")]);
    assert_eq!((heartbeats[3].to, heartbeats[3].note.as_str()), (None, "heartbeat for destination 4"));
    assert_eq!(m.status().stats.cyclic_sent, 4);
    // Started again: the next item is announced at once.
    let out = m.on_message("command", &values(&[("code", " 1 ")]), Some(1));
    assert_eq!(notes(&out), ["command 1: running = 1"]);
    let out = m.tick(10, 1);
    assert_eq!(sent(&out)[0].note, "announce 1");
    // The speed: set by a command, read by the status line at 5000 ms, settable by the knob.
    assert_eq!(notes(&m.on_message("command", &values(&[("code", "7")]), None)), ["command 7: speed = 70"]);
    assert_eq!(notes(&m.on_message("command", &values(&[("code", "5")]), None)), ["command 5: no such command"]);
    assert_eq!(notes(&m.on_message("command", &values(&[("nope", "1")]), None)), ["command : no such command"]);
    assert_eq!(m.status().stats.commands, 3);
    m.set_register("running", 0).unwrap();
    let out = run(&mut m, 4000, 1);
    let status: Vec<&Outgoing> = out.iter().filter(|o| o.message == "status").collect();
    assert_eq!(status.len(), 1);
    assert_eq!((field(status[0], "running"), field(status[0], "speed"), status[0].note.as_str()), ("0", "70", "status"));
    assert_eq!(m.status().registers, BTreeMap::from([("running".to_string(), 0), ("speed".to_string(), 70)]));
    // A cyclic message without destinations, every interval exactly, and the counter and the
    // clock as sources.
    let mut c = Controller::new(
        rules("registers = { speed = 3 }\n[[cyclic]]\nmessage = \"status\"\nevery_ms = 250\nfields = [ { name = \"running\", counter = { first = 5, last = 6 } }, { name = \"speed\", register = \"speed\" } ]\n"),
        Some(1),
    );
    let out = run(&mut c, 1000, 0);
    let s = sent(&out);
    assert_eq!(s.len(), 4, "every 250 ms, peers or none");
    assert_eq!(s.iter().map(|o| (field(o, "running"), field(o, "speed"))).collect::<Vec<_>>(), [("5", "3"), ("6", "3"), ("5", "3"), ("6", "3")], "the counter wraps after its last");
    let mut now = Controller::new(
        rules("[[cyclic]]\nmessage = \"outcome\"\nevery_ms = 10\nfields = [ { name = \"item\", now = \"%Y\" }, { name = \"dest\", cycle = [1, 2, 3] }, { name = \"result\", template = \"D{dest}\" } ]\n"),
        Some(1),
    );
    let out = run(&mut now, 30, 0);
    let s = sent(&out);
    assert_eq!(s.len(), 3);
    assert_eq!(field(s[0], "item"), chrono::Local::now().format("%Y").to_string());
    assert_eq!(s.iter().map(|o| (field(o, "dest"), field(o, "result"))).collect::<Vec<_>>(), [("1", "D1"), ("2", "D2"), ("3", "D3")]);
}

#[test]
fn the_unit_answers_with_a_decision_sticky_by_code_and_avoiding_what_is_blocked() {
    let prose = doc_prose();
    assert!(prose.contains("an item whose `code` is empty goes to destination 0, any other to destination 1, 2 or 3 in the shares 2, 1 and 1, the same destination again for the same `code`, and to 9, the reject, when every one of those is full."));
    let mut u = Controller::new(unit_rules(), None);
    let announce = |item: &str, code: &str| values(&[("item", item), ("lane", "1"), ("read", "Y"), ("code", code), ("weight", "500")]);
    // An answer to the peer the announcement came from, at once, its fields from the message.
    let out = u.on_message("announce", &announce("1", "PKG1"), Some(7));
    assert_eq!(out.len(), 1);
    let a = &out[0];
    assert_eq!((a.message.as_str(), a.to, a.note.as_str(), field(a, "item"), field(a, "flag")), ("answer", Some(7), "answer to announce", "1", "1"));
    let dest = field(a, "dest").to_string();
    assert!(["1", "2", "3"].contains(&dest.as_str()), "{dest}");
    assert_eq!(u.status().stats.answers_sent, 1);
    // The same code decides the same again; an empty code is rejected to 0.
    for _ in 0..20 {
        let out = u.on_message("announce", &announce("2", "PKG1"), Some(7));
        assert_eq!(field(&out[0], "dest"), dest);
    }
    assert_eq!(field(&u.on_message("announce", &announce("3", ""), Some(7))[0], "dest"), "0");
    assert_eq!(field(&u.on_message("announce", &announce("4", "  "), Some(7))[0], "dest"), "0");
    // The spread over many codes: every destination taken, 1 about twice as often.
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for i in 0..400 {
        let out = u.on_message("announce", &announce("9", &format!("X{i}")), Some(7));
        *counts.entry(field(&out[0], "dest").to_string()).or_default() += 1;
    }
    assert_eq!(counts.keys().collect::<Vec<_>>(), ["1", "2", "3"], "{counts:?}");
    assert!((150..=250).contains(&counts["1"]) && (60..=140).contains(&counts["2"]) && (60..=140).contains(&counts["3"]), "{counts:?}");
    // Destination 1 marked full on the unit: never drawn again, and a code that remembered 1
    // is decided afresh.
    u.set_bit(1, "full", true).unwrap();
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for i in 0..100 {
        let out = u.on_message("announce", &announce("9", &format!("Y{i}")), Some(7));
        *counts.entry(field(&out[0], "dest").to_string()).or_default() += 1;
    }
    assert_eq!(counts.keys().collect::<Vec<_>>(), ["2", "3"], "{counts:?}");
    let again = field(&u.on_message("announce", &announce("5", "PKG1"), Some(7))[0], "dest").to_string();
    if dest == "1" {
        assert_ne!(again, "1");
    } else {
        assert_eq!(again, dest);
    }
    // Every chute full: no rule matches a read code and nothing can be drawn, so the reject;
    // an empty code still takes its rule.
    u.set_bit_everywhere("full", true).unwrap();
    assert_eq!(field(&u.on_message("announce", &announce("6", "Z1"), Some(7))[0], "dest"), "9");
    assert_eq!(field(&u.on_message("announce", &announce("7", "PKG1"), Some(7))[0], "dest"), "9", "a remembered chute that is full is not taken");
    assert_eq!(field(&u.on_message("announce", &announce("8", ""), Some(7))[0], "dest"), "0");
    u.set_bit_everywhere("full", false).unwrap();
    assert_ne!(field(&u.on_message("announce", &announce("9", "Z2"), Some(7))[0], "dest"), "9");
    // A reject is never remembered: the code decided while every chute was full draws again,
    // and the code remembered before keeps its chute.
    assert!(["1", "2", "3"].contains(&field(&u.on_message("announce", &announce("10", "Z1"), Some(7))[0], "dest")), "a code rejected while full draws again");
    assert_eq!(field(&u.on_message("announce", &announce("11", "PKG1"), Some(7))[0], "dest"), again, "the chute remembered before the full period");
    // An answer whose field cannot render is not sent, and counted: a rule-only decision with
    // nothing matching and no reject.
    let mut strict = Controller::new(
        rules("[[answers]]\non = \"announce\"\nsend = \"answer\"\nafter_ms = 30\nfields = [ { name = \"item\", from = \"item\" }, { name = \"dest\", decide = { rules = [ { field = \"code\", pattern = \"^A\", value = 1 } ] } }, { name = \"flag\", from = \"nope\" } ]\n"),
        Some(1),
    );
    let out = strict.on_message("announce", &announce("1", "B1"), Some(7));
    assert_eq!(notes(&out), ["answer to announce not sent: `dest`: no decision"]);
    assert_eq!(strict.status().stats.answers_refused, 1);
    let out = strict.on_message("announce", &announce("1", "A1"), Some(7));
    assert_eq!(notes(&out), ["answer to announce not sent: `flag`: nothing named `nope` to read from"]);
    assert_eq!(strict.status().stats.answers_refused, 2);
    // A delayed answer goes out when its delay has passed.
    let mut late = Controller::new(
        rules("[[answers]]\non = \"announce\"\nsend = \"answer\"\nafter_ms = 30\nfields = [ { name = \"item\", from = \"item\" }, { name = \"dest\", value = 2 }, { name = \"flag\", value = 1 } ]\n"),
        Some(1),
    );
    assert!(sent(&late.on_message("announce", &announce("1", "A1"), Some(7))).is_empty());
    assert!(sent(&late.tick(20, 1)).is_empty());
    let out = late.tick(10, 1);
    assert_eq!((sent(&out).len(), sent(&out)[0].to, late.status().stats.answers_sent), (1, Some(7), 1));
}

#[test]
fn faults_mark_their_share_of_items_and_the_rate_is_the_interval_s_whatever_the_tick() {
    let prose = doc_prose();
    assert!(prose.contains("one item in 10 carries the fault `noread` (its `read` flag `N` and an empty `code`)"));
    let mut m = Controller::new(machine_rules(), Some(3));
    m.set_interval(10).unwrap();
    let out = run(&mut m, 10_000, 1);
    let items = announces(&out);
    assert_eq!(items.len(), 1000, "one every 10 ms for 10 s");
    let faulted: Vec<&&Outgoing> = items.iter().filter(|o| field(o, "read") == "N").collect();
    assert!((50..=150).contains(&faulted.len()), "about one in ten: {}", faulted.len());
    assert!(faulted.iter().all(|o| field(o, "code").is_empty()), "the fault sets every field it names");
    assert_eq!(m.status().stats.faults, BTreeMap::from([("noread".to_string(), faulted.len() as u64)]));
    // The share turned live: all, then none.
    m.set_fault("noread", 100).unwrap();
    assert!(announces(&run(&mut m, 200, 1)).iter().all(|o| field(o, "read") == "N" && field(o, "code").is_empty()));
    m.set_fault("noread", 0).unwrap();
    assert!(announces(&run(&mut m, 200, 1)).iter().all(|o| field(o, "read") == "Y" && field(o, "code").starts_with("PKG")));
    assert_eq!(m.status().faults, BTreeMap::from([("noread".to_string(), 0)]));
    // The rate is the interval's, whatever the tick: 100 ms items over 10 s are 100 on ticks
    // of 10 ms, of 30 ms and of 70 ms alike, and 20 ms items on a 31 ms tick (the Windows
    // quantum) are 500 too, a pass announcing every item due since the last.
    for (tick, interval) in [(10, 100), (30, 100), (70, 100), (31, 20), (250, 100)] {
        let mut m = Controller::new(machine_rules(), Some(1));
        m.set_interval(interval).unwrap();
        let mut count = 0;
        let mut elapsed = 0;
        count += announces(&m.tick(0, 1)).len();
        while elapsed < 10_000 {
            count += announces(&m.tick(tick, 1)).len();
            elapsed += tick;
        }
        let expected = (1 + elapsed / interval) as usize;
        assert!((expected - 1..=expected + 1).contains(&count), "tick {tick} at {interval} ms: {count} items in {elapsed} ms, not {expected}");
    }
    // A stall: ten intervals announce ten; fifty announce twenty (the burst's most) and skip
    // the rest, the phase kept; the first item after an idle comes at once, not as a burst.
    let mut m = Controller::new(machine_rules(), Some(1));
    m.tick(0, 1);
    assert_eq!(announces(&m.tick(1000, 1)).len(), 10, "a stall of ten intervals announces the ten due");
    assert_eq!(announces(&m.tick(100, 1)).len(), 1, "and the next is due an interval after the last that was due");
    assert!(announces(&m.tick(50, 1)).is_empty());
    assert_eq!(announces(&m.tick(50, 1)).len(), 1);
    assert_eq!(announces(&m.tick(5000, 1)).len(), MAX_BURST, "a stall of fifty intervals announces the burst's most");
    assert_eq!(announces(&m.tick(100, 1)).len(), 1, "and the phase is kept");
    assert!(announces(&m.tick(50, 1)).is_empty());
    assert_eq!(announces(&m.tick(50, 1)).len(), 1);
    assert!(announces(&m.tick(3000, 0)).is_empty());
    assert_eq!(announces(&m.tick(0, 1)).len(), 1, "after an idle with no peer, one item at once, not thirty");
    assert!(announces(&m.tick(99, 1)).is_empty());
    assert_eq!(announces(&m.tick(1, 1)).len(), 1);
    // The interval turned live: the next item comes within the new interval.
    m.set_interval(20).unwrap();
    assert_eq!(announces(&run(&mut m, 200, 1)).len(), 10);
    // Announce now, whatever the interval; and a seed repeats a run exactly.
    let a = m.announce_now().unwrap();
    assert_eq!(a.message, "announce");
    let mut one = Controller::new(machine_rules(), Some(42));
    let mut two = Controller::new(machine_rules(), Some(42));
    let (x, y) = (run(&mut one, 3000, 1), run(&mut two, 3000, 1));
    assert_eq!(x, y);
    let mut other = Controller::new(machine_rules(), Some(43));
    assert_ne!(run(&mut other, 3000, 1), x, "another seed, other draws");
    // The history keeps the newest 200 ended items.
    let mut p = Controller::new(machine_rules(), Some(1));
    p.set_fault("noread", 0).unwrap();
    p.set_interval(10).unwrap();
    for i in 1..=250 {
        p.tick(10, 1);
        p.on_message("answer", &values(&[("item", &i.to_string()), ("dest", "1"), ("flag", "3")]), None);
    }
    let s = p.status();
    assert_eq!((s.history.len(), s.history[0].key.as_str(), s.history[199].key.as_str(), s.stats.parked), (200, "250", "51", 250));
}

// ---------------------------------------------------------------------------------------------
// Over the wire
// ---------------------------------------------------------------------------------------------

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn channel(name: &str, role: Role, controller: Rules) -> BenchChannel {
    start(ChannelConfig { name: name.into(), role, spec: spec(), catalog: catalog(), messages: Vec::new(), controller: Some(controller), seed: Some(1) }).unwrap()
}

/// Wait up to `within` for `what` to hold, polling the status.
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

#[test]
fn the_machine_and_the_unit_converse_over_a_real_socket_and_the_knobs_turn_live() {
    let port = free_port();
    let machine = channel("machine", Role::Listen { bind: "127.0.0.1".into(), port }, machine_rules());
    let unit = channel("unit", Role::Connect { host: "127.0.0.1".into(), port }, unit_rules());
    assert!(wait(Duration::from_secs(5), || machine.status().peers.len() == 1 && unit.status().peers.len() == 1), "the link: {:?} / {:?}", machine.status().wire, unit.status().wire);
    // Items flow: announced by the machine, answered by the unit, outcomes reported back,
    // heartbeats per destination, every message decoded on the other side.
    assert!(
        wait(Duration::from_secs(5), || {
            let m = machine.status().controller.unwrap().stats;
            let u = unit.status();
            m.announced >= 10 && m.answered >= 8 && m.ended.values().sum::<u64>() >= 3 && u.received.get("heartbeat").copied().unwrap_or(0) >= 4 && u.received.get("outcome").copied().unwrap_or(0) >= 3
        }),
        "machine {:?}\nunit {:?}",
        machine.status().controller.unwrap().stats,
        unit.status()
    );
    let m = machine.status();
    let u = unit.status();
    // Two snapshots of two ends of a live link: what one sent, the other has received, or is
    // about to.
    let near = |received: Option<&u64>, sent: Option<&u64>| {
        let (r, s) = (received.copied().unwrap_or(0), sent.copied().unwrap_or(0));
        r <= s && r + 2 >= s
    };
    assert_eq!(m.sent.get("announce"), Some(&m.controller.as_ref().unwrap().stats.announced), "last error {:?}", m.last_error);
    assert!(near(u.received.get("announce"), m.sent.get("announce")), "{:?} / {:?}", u.received, m.sent);
    assert_eq!(u.sent.get("answer"), Some(&u.controller.as_ref().unwrap().stats.answers_sent));
    assert!(near(m.received.get("answer"), u.sent.get("answer")), "{:?} / {:?}", m.received, u.sent);
    assert!(m.received.get("answer").copied().unwrap_or(0) >= 8, "{m:?}");
    assert_eq!((m.unknown_ids, m.undecodable, u.unknown_ids, u.undecodable), (0, 0, 0, 0));
    let answer = u.recent.iter().find(|l| l.direction == Direction::Out && l.message == "answer").expect("an answer in the unit's monitor");
    assert!(answer.fields.contains_key("item") && answer.fields.contains_key("dest") && answer.fields["flag"] == "1", "{answer:?}");
    let outcome = u.recent.iter().find(|l| l.direction == Direction::In && l.message == "outcome").expect("an outcome in the unit's monitor");
    assert!(["SORTED", "LOST"].contains(&outcome.fields["result"].as_str()), "{outcome:?}");
    assert_eq!(outcome.preview, format!("ITEM={};DEST={};RESULT={}", outcome.fields["item"], outcome.fields["dest"], outcome.fields["result"]));
    let heartbeat = u.recent.iter().find(|l| l.direction == Direction::In && l.message == "heartbeat").expect("a heartbeat");
    assert!(["1", "2", "3", "4"].contains(&heartbeat.fields["dest"].as_str()));
    assert!(m.controller.as_ref().unwrap().oldest_unanswered_ms.unwrap_or(0) < 1000, "the unit answers at once: {:?}", m.controller.as_ref().unwrap().oldest_unanswered_ms);
    let latencies: Vec<u64> = m.controller.as_ref().unwrap().history.iter().filter_map(|i| i.reply_latency_ms()).collect();
    assert!(!latencies.is_empty() && latencies.iter().all(|l| *l < 500), "{latencies:?}");

    // Every chute full: every outcome FULL, every item round again.
    machine.set_bit_everywhere("full", true).unwrap();
    let before = machine.status().controller.unwrap().stats.ended.get("blocked:full").copied().unwrap_or(0);
    assert!(wait(Duration::from_secs(5), || machine.status().controller.unwrap().stats.ended.get("blocked:full").copied().unwrap_or(0) >= before + 3));
    assert!(machine.status().controller.unwrap().stats.requeued >= 3);
    let full = unit.status().recent.iter().find(|l| l.message == "outcome" && l.fields["result"] == "FULL").is_some();
    assert!(full, "a FULL outcome reached the unit");
    machine.set_bit_everywhere("full", false).unwrap();
    assert_eq!(machine.status().controller.unwrap().destinations, BTreeMap::from([(1, 0), (2, 0), (3, 0), (4, 0)]));

    // Stopped by the register, started again by a command the unit sends.
    machine.set_register("running", 0).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    let stopped = machine.status().controller.unwrap().stats.announced;
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(machine.status().controller.unwrap().stats.announced, stopped, "nothing announced while stopped");
    unit.send_now("command", values(&[("code", "1")]), None);
    assert!(wait(Duration::from_secs(5), || machine.status().controller.unwrap().registers["running"] == 1 && machine.status().controller.unwrap().stats.announced > stopped));
    assert_eq!(machine.status().controller.unwrap().stats.commands, 1);
    assert!(machine.status().recent.iter().any(|l| l.direction == Direction::Note && l.preview == "command 1: running = 1"));
    // The faults live, measured on the wire.
    machine.set_fault("noread", 100).unwrap();
    let before = unit.status().received.get("answer").copied().unwrap_or(0);
    assert!(wait(Duration::from_secs(5), || {
        let u = unit.status();
        u.recent.iter().filter(|l| l.direction == Direction::In && l.message == "announce").take(3).all(|l| l.fields["read"] == "N" && l.fields["code"].is_empty()) && u.received.get("announce").copied().unwrap_or(0) >= before + 3
    }));
    assert!(unit.status().recent.iter().any(|l| l.direction == Direction::Out && l.message == "answer" && l.fields["dest"] == "0"), "an empty code is rejected to 0");
    // The knobs through the channel refuse what the rules do not have, and a channel without
    // a controller refuses them all.
    assert_eq!(machine.set_bit(9, "full", true).unwrap_err(), "no destination 9");
    assert_eq!(unit.set_interval(10).unwrap_err(), "no items to announce");
    let plain = start(ChannelConfig { name: "plain".into(), role: Role::Listen { bind: "127.0.0.1".into(), port: free_port() }, spec: spec(), catalog: catalog(), messages: Vec::new(), controller: None, seed: None }).unwrap();
    assert_eq!(plain.announce_now().unwrap_err(), "this channel has no controller");
    assert!(plain.status().controller.is_none());
    // Stopped: the unit sees the machine go, the port is free again.
    machine.stop();
    assert!(wait(Duration::from_secs(5), || unit.status().peers.is_empty()));
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(), "the port is free");
    unit.stop();
    plain.stop();
}
