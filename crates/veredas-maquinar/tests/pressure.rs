//! Controllable pressure (contract: the pressure clause): the machine and the unit of
//! `docs/controller-schema.md`'s worked example over a real loopback socket, with the rate, the
//! faults, every chute full and the no-reads turned live from the bench's handles and measured
//! on the wire.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use veredas_core::telegram::Role;
use veredas_core::telegram::catalog::{self, Catalog};
use veredas_core::telegram::schema::{ChannelSpec, load as load_spec};
use veredas_maquinar::channel::{BenchChannel, ChannelConfig, Direction, start};
use veredas_maquinar::controller::{Rules, load};

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

fn spec() -> ChannelSpec {
    load_spec(&doc_blocks("channel-schema.md")[0]).unwrap()
}

fn catalog() -> Catalog {
    catalog::load(&doc_blocks("controller-schema.md")[0]).unwrap()
}

fn rules(i: usize) -> Rules {
    load(&doc_blocks("controller-schema.md")[i], &catalog()).unwrap()
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn channel(name: &str, role: Role, controller: Rules) -> BenchChannel {
    start(ChannelConfig { name: name.into(), role, spec: spec(), catalog: catalog(), messages: Vec::new(), controller: Some(controller), seed: Some(5) }).unwrap()
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

fn announced(m: &BenchChannel) -> u64 {
    m.status().controller.unwrap().stats.announced
}

/// How many items the machine announced over `window`, measured on the unit's side.
fn rate(unit: &BenchChannel, window: Duration) -> u64 {
    let before = unit.status().received.get("announce").copied().unwrap_or(0);
    std::thread::sleep(window);
    unit.status().received.get("announce").copied().unwrap_or(0) - before
}

#[test]
fn the_rate_the_faults_the_full_chutes_and_the_no_reads_are_turned_live_and_measured_on_the_wire() {
    let port = free_port();
    let machine = channel("machine", Role::Listen { bind: "127.0.0.1".into(), port }, rules(1));
    let unit = channel("unit", Role::Connect { host: "127.0.0.1".into(), port }, rules(2));
    machine.set_fault("noread", 0).unwrap();
    assert!(wait(Duration::from_secs(5), || machine.status().peers.len() == 1 && unit.status().peers.len() == 1));
    assert!(wait(Duration::from_secs(5), || announced(&machine) >= 3));

    // The rate: one item every 100 ms as configured, 20 in 2 s, then every 50 ms (40 in 2 s),
    // every 20 ms (100 in 2 s, one a tick now that the bench holds the timer fine: a pass announces every item
    // due since the last), then every 400 ms (5 in 2 s), each the interval's whatever the tick.
    let slow = rate(&unit, Duration::from_secs(2));
    assert!((17..=23).contains(&slow), "{slow} items in 2 s at 100 ms");
    machine.set_interval(50).unwrap();
    assert_eq!(machine.status().controller.unwrap().interval_ms, Some(50));
    let fast = rate(&unit, Duration::from_secs(2));
    assert!((34..=46).contains(&fast), "{fast} items in 2 s at 50 ms");
    machine.set_interval(20).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let faster = rate(&unit, Duration::from_secs(2));
    assert!((85..=115).contains(&faster), "{faster} items in 2 s at 20 ms");
    machine.set_interval(400).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    let slower = rate(&unit, Duration::from_secs(2));
    assert!((4..=6).contains(&slower), "{slower} items in 2 s at 400 ms");
    // Announce now: exactly one more, with the interval too long for a regular item to come.
    machine.set_interval(60_000).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    let before = unit.status().received.get("announce").copied().unwrap_or(0);
    machine.announce_now().unwrap();
    assert!(wait(Duration::from_millis(500), || unit.status().received.get("announce").copied().unwrap_or(0) == before + 1));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(unit.status().received.get("announce").copied().unwrap_or(0), before + 1, "one, not a burst");

    // The faults: half the items no-reads over a thousand items at 10 ms, counted by the
    // machine and seen on the wire by the unit; every no-read, and nothing else, rejected to 0.
    machine.set_interval(10).unwrap();
    let s0 = machine.status().controller.unwrap().stats;
    machine.set_fault("noread", 50).unwrap();
    assert_eq!(machine.status().controller.unwrap().faults["noread"], 50);
    assert!(wait(Duration::from_secs(40), || announced(&machine) >= s0.announced + 1000));
    machine.set_fault("noread", 0).unwrap();
    let s1 = machine.status().controller.unwrap().stats;
    let total = s1.announced - s0.announced;
    let noreads = s1.faults["noread"] - s0.faults.get("noread").copied().unwrap_or(0);
    assert!((44..=56).contains(&(noreads * 100 / total)), "{noreads} no-reads in {total} at 50 %");
    let u = unit.status();
    let items: Vec<_> = u.recent.iter().filter(|l| l.direction == Direction::In && l.message == "announce").collect();
    assert!(items.len() >= 20, "{}", items.len());
    let seen = items.iter().filter(|l| l.fields["read"] == "N" && l.fields["code"].is_empty()).count();
    assert!((15..=85).contains(&(seen * 100 / items.len())), "{seen} no-reads in the {} the unit still shows", items.len());
    assert!(u.recent.iter().any(|l| l.direction == Direction::Out && l.message == "answer" && l.fields["dest"] == "0"), "{:?}", u.recent);
    // Once every item is answered (nothing new announced meanwhile): as many invalid
    // destinations as no-reads, no more, no fewer.
    machine.set_interval(60_000).unwrap();
    assert!(wait(Duration::from_secs(5), || machine.status().controller.unwrap().awaiting.is_empty()), "{:?}", machine.status().controller.unwrap().awaiting);
    let s2 = machine.status().controller.unwrap().stats;
    assert_eq!(s2.invalid_destinations - s0.invalid_destinations, noreads, "{s2:?}");

    // Every chute full: every outcome FULL and every item round again; then every chute back
    // and the outcomes drawn again.
    machine.set_interval(20).unwrap();
    machine.set_bit_everywhere("full", true).unwrap();
    let s = machine.status().controller.unwrap();
    assert!(s.destinations.values().all(|w| *w & 1 == 1), "{:?}", s.destinations);
    let full_before = s.stats.ended.get("blocked:full").copied().unwrap_or(0);
    let requeued_before = s.stats.requeued;
    // An item rejected to destination 0 is at no chute: its outcome is drawn, not forced.
    let at_a_chute = |u: &veredas_maquinar::channel::ChannelStatus| -> Vec<String> {
        u.recent.iter().filter(|l| l.direction == Direction::In && l.message == "outcome" && l.fields["dest"] != "0").take(5).map(|l| l.fields["result"].clone()).collect()
    };
    assert!(wait(Duration::from_secs(5), || {
        let s = machine.status().controller.unwrap().stats;
        let seen = at_a_chute(&unit.status());
        s.ended.get("blocked:full").copied().unwrap_or(0) >= full_before + 5 && s.requeued >= requeued_before + 5 && seen.len() == 5 && seen.iter().all(|r| r == "FULL")
    }), "{:?}\n{:?}", machine.status().controller.unwrap().stats, at_a_chute(&unit.status()));
    machine.set_bit_everywhere("full", false).unwrap();
    assert!(machine.status().controller.unwrap().destinations.values().all(|w| *w == 0));
    let full_after = machine.status().controller.unwrap().stats.ended.get("blocked:full").copied().unwrap_or(0);
    assert!(wait(Duration::from_secs(5), || {
        let u = unit.status();
        u.recent.iter().filter(|l| l.direction == Direction::In && l.message == "outcome" && l.fields["dest"] != "0").take(3).all(|l| l.fields["result"] != "FULL")
    }));
    std::thread::sleep(Duration::from_millis(700));
    assert!(machine.status().controller.unwrap().stats.ended.get("blocked:full").copied().unwrap_or(0) <= full_after + 2, "items answered while full may still end FULL; no more");
    // The lap (2000 ms) brings the requeued items back: their laps count.
    assert!(wait(Duration::from_secs(4), || {
        let s = machine.status().controller.unwrap();
        s.awaiting.iter().any(|i| i.laps >= 2) || s.history.iter().any(|i| i.laps >= 2)
    }), "{:?}", machine.status().controller.unwrap().awaiting);

    // The peer gone: nothing more is announced, and the oldest unanswered item ages.
    unit.stop();
    assert!(wait(Duration::from_secs(5), || machine.status().peers.is_empty()));
    let stopped = announced(&machine);
    let aged = machine.status().controller.unwrap().oldest_unanswered_ms;
    std::thread::sleep(Duration::from_millis(600));
    let s = machine.status().controller.unwrap();
    assert_eq!(s.stats.announced, stopped, "announced into nothing: none");
    assert!(s.oldest_unanswered_ms.unwrap_or(0) >= aged.unwrap_or(0) + 500 || s.awaiting.is_empty(), "{:?} then {:?}", aged, s.oldest_unanswered_ms);
    // A register set by hand, as a command would.
    machine.set_register("running", 0).unwrap();
    assert_eq!(machine.status().controller.unwrap().registers["running"], 0);
    let _ = HashMap::<String, String>::new();
    machine.stop();
}
