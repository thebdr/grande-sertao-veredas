//! Asking (contract: the asking clause): a DCP Identify sent through a listener already hearing
//! its interface, from that interface's MAC, paced like any frame it sends; its answers the
//! Identify answers kept from that interface to that MAC under its transaction id, fused by the
//! listener; refused where it cannot be heard, sent or bounded. On the memory hub, a device of the
//! test's own answering; and on a bridge in a private network, against the bench's PROFINET
//! devices (maquinar's executable), a staged duplicate name among them.

mod common;

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use veredas_aranha::ask::{self, AskError, Identify};
use veredas_aranha::listen::{ListenError, Listener, Options};
use veredas_aranha::profinet::dcp::{self, DeviceIdentity, Filter, IpSuite, Service};
use veredas_aranha::profinet::ether::{self, ETHERTYPE_PROFINET};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::wire::hub::{Hub, HubPort};
use veredas_aranha::wire::pcap::Replay;
use veredas_aranha::wire::{Frame, Wire, WireError};

const DEV: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
const OTHER: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x0B]);
const LAN0: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0xA0]);
const LAN1: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0xA1]);

fn identity() -> DeviceIdentity {
    DeviceIdentity {
        name: Some("io-device-1".into()),
        alias: Some("port-001.switch-1".into()),
        ip: Some(IpSuite { ip: Ipv4Addr::new(192, 168, 0, 10), mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::UNSPECIFIED }),
        vendor_id: Some(0x1234),
        device_id: Some(0x0203),
        ..Default::default()
    }
}

/// How the test's device answers an Identify it matches.
#[derive(Clone, Copy, Default)]
struct Manner {
    /// Before answering.
    delay: Duration,
    /// Besides its answer, an answer under another transaction id to the asker, and one under the
    /// same id to another station.
    distract: bool,
    /// Hellos sent after the answer, 50 ms later.
    flood: usize,
}

/// A device on a hub port answering the Identify requests it matches, until stopped.
struct Device {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Device {
    fn start(mut port: HubPort, manner: Manner) -> Device {
        let stop = Arc::new(AtomicBool::new(false));
        let halt = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            let id = identity();
            while !halt.load(Ordering::SeqCst) {
                let Ok(Some(f)) = port.recv(Duration::from_millis(20)) else { continue };
                let Ok(e) = ether::parse(&f.bytes) else { continue };
                if e.ethertype != ETHERTYPE_PROFINET {
                    continue;
                }
                let Ok(pdu) = dcp::decode(e.payload) else { continue };
                if pdu.service != Service::Identify || pdu.service_type.is_response() {
                    continue;
                }
                let asked = DeviceIdentity::from_blocks(&pdu.blocks);
                let matches = match (&asked.name, &asked.alias) {
                    (Some(n), _) => id.name.as_ref() == Some(n),
                    (_, Some(a)) => id.alias.as_ref() == Some(a),
                    _ => true,
                };
                if !matches {
                    continue;
                }
                std::thread::sleep(manner.delay);
                port.send(&dcp::identify_response(e.src, DEV, pdu.xid, &id).unwrap()).unwrap();
                if manner.distract {
                    port.send(&dcp::identify_response(e.src, DEV, pdu.xid.wrapping_add(1), &id).unwrap()).unwrap();
                    port.send(&dcp::identify_response(OTHER, DEV, pdu.xid, &id).unwrap()).unwrap();
                    // Under its xid to the asker, but no Identify answer: another frame id, and a
                    // request's service type.
                    let mut other_id = dcp::decode(ether::parse(&dcp::identify_response(e.src, DEV, pdu.xid, &id).unwrap()).unwrap().payload).unwrap();
                    other_id.frame_id = dcp::FRAME_ID_GET_SET;
                    port.send(&other_id.frame(e.src, DEV).unwrap()).unwrap();
                    let name = dcp::Block { option: 2, suboption: 2, info: None, body: dcp::BlockBody::NameOfStation { name: "io-device-1".into() } };
                    let asking = dcp::DcpPdu {
                        frame_id: dcp::FRAME_ID_IDENTIFY_RES,
                        service: Service::Identify,
                        service_type: dcp::ServiceType::Request,
                        xid: pdu.xid,
                        response_delay: 0,
                        blocks: vec![name],
                        trailing: Vec::new(),
                    };
                    port.send(&asking.frame(e.src, DEV).unwrap()).unwrap();
                    // Under DCP's answer frame id, its xid, to the asker: a Get's answer, and an
                    // Identify answer under another EtherType.
                    let mut get = asking.clone();
                    get.service = Service::Get;
                    get.service_type = dcp::ServiceType::Success;
                    get.blocks[0].info = Some(0);
                    port.send(&get.frame(e.src, DEV).unwrap()).unwrap();
                    let mut elsewhere = dcp::identify_response(e.src, DEV, pdu.xid, &id).unwrap();
                    elsewhere[12..14].copy_from_slice(&0x88B6u16.to_be_bytes());
                    port.send(&elsewhere).unwrap();
                }
                if manner.flood > 0 {
                    std::thread::sleep(Duration::from_millis(50));
                    for i in 0..manner.flood {
                        port.send(&dcp::hello(DEV, i as u32, &id).unwrap()).unwrap();
                    }
                }
            }
        });
        Device { stop, thread: Some(thread) }
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn quick(filter: Filter) -> Identify {
    Identify { filter, response_delay: 1, wait: Duration::from_millis(300) }
}

#[test]
fn an_identify_is_answered_by_what_it_matches_and_only_its_answers_count() {
    let hub = Hub::new();
    let _dev = Device::start(hub.port("dev", DEV), Manner { distract: true, ..Manner::default() });
    // A second interface of this machine on the same hub hears the answer too: not lan0's.
    let l = Arc::new(Listener::start(vec![Box::new(hub.port("lan0", LAN0)), Box::new(hub.port("lan1", LAN1))], Options::default()).unwrap());
    // An Identify answer this machine itself sends while it waits is no answer, nor another's.
    hub.set_now(77_000);
    let sender = {
        let l = Arc::clone(&l);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            l.send("lan0", dcp::identify_response(OTHER, LAN0, 9, &identity()).unwrap()).unwrap();
        })
    };
    let asked = ask::identify(&l, "lan0", &quick(Filter::All)).unwrap();
    sender.join().unwrap();
    assert_eq!(l.interface("lan0").unwrap().sent, 2);
    // The request as kept: sent on lan0 from its MAC to DCP's group address, an Identify of every
    // device under the asked transaction id and delay.
    let req = l.frame(asked.request).unwrap();
    assert!(req.sent);
    assert_eq!((req.frame.iface.as_str(), req.frame.at_ns), ("lan0", asked.sent_ns));
    let e = ether::parse(&req.frame.bytes).unwrap();
    assert_eq!((e.src, e.dst), (LAN0, Mac::DCP_MULTICAST));
    let pdu = dcp::decode(e.payload).unwrap();
    assert_eq!((pdu.service, pdu.service_type.is_response(), pdu.xid, pdu.response_delay), (Service::Identify, false, asked.xid, 1));
    assert_eq!(req.frame.bytes, dcp::identify_request(LAN0, asked.xid, 1, &Filter::All).unwrap());
    // Its one answer, after it, with what the device said; the two not to it counted apart.
    assert_eq!(asked.iface, "lan0");
    assert_eq!(asked.answers.len(), 1, "{asked:?}");
    let a = &asked.answers[0];
    assert_eq!((a.mac, a.identity.clone()), (DEV, DeviceIdentity::from_blocks(&identity().blocks())));
    let kept = l.frame(a.frame).unwrap();
    assert!(a.frame > asked.request);
    assert_eq!((kept.frame.iface.as_str(), kept.sent, a.at_ns), ("lan0", false, 77_000), "stamped by the hub's clock");
    assert_eq!((asked.others, asked.lost), (2, 0));
    // The listener fused it, heard on both interfaces, its name first witnessed by a frame of the
    // device's (which only answers): its answer kept on lan0 after the request, or that answer's
    // copy on lan1, which lan1's worker may keep while lan0's, having handed the request to the
    // wire, has yet to keep it as sent (the order kept across interfaces is no order of cause).
    let s = l.snapshot();
    let dev = &s.fusion.devices[&DEV];
    assert_eq!(dev.name.current().map(String::as_str), Some("io-device-1"));
    let first = dev.name.candidates[0].first.frame;
    let witness = l.frame(first).unwrap();
    assert!(!witness.sent && ether::parse(&witness.frame.bytes).unwrap().src == DEV, "{witness:?}");
    assert!(first > asked.request || witness.frame.iface == "lan1", "the name first witnessed at {first}, on {}, the request at {}", witness.frame.iface, asked.request);
    assert_eq!(dev.heard_on.iter().cloned().collect::<Vec<_>>(), ["lan0", "lan1"]);
    assert_eq!(s.stats.interfaces["lan1"].heard, 9, "the request, the answer, the six not to it and the answer lan0 sent");
    // By station name and by alias: answered; by a name it does not hold: nothing.
    for filter in [Filter::Name("io-device-1".into()), Filter::Alias("port-001.switch-1".into())] {
        let again = ask::identify(&l, "lan0", &quick(filter.clone())).unwrap();
        assert_eq!(again.answers.iter().map(|a| a.mac).collect::<Vec<_>>(), [DEV], "{filter:?}");
        assert_ne!(again.xid, asked.xid);
        assert_eq!(l.frame(again.request).unwrap().frame.bytes, dcp::identify_request(LAN0, again.xid, 1, &filter).unwrap());
    }
    let none = ask::identify(&l, "lan0", &quick(Filter::Name("io-device-2".into()))).unwrap();
    assert_eq!((none.answers.len(), none.others), (0, 0));
}

#[test]
fn an_identify_waits_its_turn_the_spread_and_its_wait_and_no_longer() {
    let hub = Hub::new();
    let _dev = Device::start(hub.port("dev", DEV), Manner::default());
    let l = Listener::start(vec![Box::new(hub.port("lan0", LAN0))], Options { rate: 1, ..Options::default() }).unwrap();
    // Three frames queued before it at one a second: it goes fourth, three gaps after the first.
    let arp = |last| veredas_aranha::observe::arp_frame(LAN0, Ipv4Addr::new(192, 168, 0, 1), Mac::BROADCAST, Ipv4Addr::new(192, 168, 0, last), false).unwrap();
    for last in [2, 3, 4] {
        l.send("lan0", arp(last)).unwrap();
    }
    let start = Instant::now();
    let asked = ask::identify(&l, "lan0", &Identify { filter: Filter::All, response_delay: 20, wait: Duration::from_millis(100) }).unwrap();
    let took = start.elapsed();
    let sent: Vec<_> = l.frames().into_iter().filter(|k| k.sent).collect();
    assert_eq!(sent.len(), 4);
    assert_eq!(sent[3].frame.at_ns, asked.sent_ns);
    assert!(asked.sent_ns - sent[0].frame.at_ns >= 3 * 1_000_000_000 - 2_000_000, "{:?}", sent.iter().map(|k| k.frame.at_ns).collect::<Vec<_>>());
    // Then the spread (20 × 10 ms) and the wait (100 ms): at least 0.3 s after it was sent.
    assert!(took >= Duration::from_millis(3000 + 300 - 5), "{took:?}");
    assert_eq!(asked.window, Duration::from_millis(300), "the spread, 20 steps of 10 ms, and the wait");
    assert!(took <= Duration::from_millis(3000 + 300 + 700), "{took:?}");
    assert_eq!(asked.answers.len(), 1);
    // An answer that comes after the spread and the wait is not this request's: heard and fused
    // by the listener all the same.
    let hub = Hub::new();
    let _late = Device::start(hub.port("dev", DEV), Manner { delay: Duration::from_millis(400), ..Manner::default() });
    let l = Listener::start(vec![Box::new(hub.port("lan0", LAN0))], Options::default()).unwrap();
    let asked = ask::identify(&l, "lan0", &quick(Filter::All)).unwrap();
    assert_eq!(asked.answers.len(), 0);
    assert!(l.until(Duration::from_secs(5), |s| s.fusion.devices.contains_key(&DEV)));
}

#[test]
fn answers_let_go_past_the_listener_s_bound_are_counted_lost() {
    let hub = Hub::new();
    let _dev = Device::start(hub.port("dev", DEV), Manner { distract: true, flood: 10, ..Manner::default() });
    let l = Listener::start(vec![Box::new(hub.port("lan0", LAN0))], Options { max_frames: 3, ..Options::default() }).unwrap();
    let asked = ask::identify(&l, "lan0", &Identify { filter: Filter::All, response_delay: 1, wait: Duration::from_millis(500) }).unwrap();
    // The answer, the six not to it and ten Hellos came after the request; three are kept.
    assert_eq!(l.kept(), (asked.request + 15, asked.request + 18));
    assert_eq!((asked.lost, asked.answers.len(), asked.others), (14, 0, 0));
    // The listener heard and fused the answer as it came.
    assert_eq!(l.snapshot().fusion.devices[&DEV].name.current().map(String::as_str), Some("io-device-1"));
}

/// A wire with no MAC of its own that hears nothing and ends never.
struct Nameless;
impl Wire for Nameless {
    fn name(&self) -> &str {
        "nameless"
    }
    fn mac(&self) -> Option<Mac> {
        None
    }
    fn send(&mut self, _: &[u8]) -> Result<(), WireError> {
        Ok(())
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        std::thread::sleep(timeout);
        Ok(None)
    }
}

/// A wire with a MAC that refuses every frame it is given to send.
struct Refusing;
impl Wire for Refusing {
    fn name(&self) -> &str {
        "refusing"
    }
    fn mac(&self) -> Option<Mac> {
        Some(LAN0)
    }
    fn send(&mut self, frame: &[u8]) -> Result<(), WireError> {
        Err(WireError::TooLong(frame.len() * 1000))
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        std::thread::sleep(timeout);
        Ok(None)
    }
}

/// A wire whose every send takes two seconds.
/// A wire whose every send takes two seconds, counting the frames it is handed.
struct CountingStall {
    handed: Arc<std::sync::atomic::AtomicUsize>,
}
impl Wire for CountingStall {
    fn name(&self) -> &str {
        "stalling"
    }
    fn mac(&self) -> Option<Mac> {
        Some(LAN0)
    }
    fn send(&mut self, _: &[u8]) -> Result<(), WireError> {
        self.handed.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_secs(2));
        Ok(())
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        std::thread::sleep(timeout);
        Ok(None)
    }
}

/// A wire that, handed its first frame, takes it until `panic_after` has passed and then panics,
/// saying when it began.
struct PanicsLate {
    name: String,
    began: Arc<std::sync::Mutex<Option<Instant>>>,
    panic_after: Duration,
}
impl Wire for PanicsLate {
    fn name(&self) -> &str {
        &self.name
    }
    fn mac(&self) -> Option<Mac> {
        Some(LAN0)
    }
    fn send(&mut self, _: &[u8]) -> Result<(), WireError> {
        let start = Instant::now();
        *self.began.lock().unwrap() = Some(start);
        std::thread::sleep(self.panic_after);
        panic!("the card fell out")
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        std::thread::sleep(timeout);
        Ok(None)
    }
}

/// A wire that answers the Identify it was last handed on every read, at once: a device flooding.
struct Answering {
    xid: Option<u32>,
}
impl Wire for Answering {
    fn name(&self) -> &str {
        "answering"
    }
    fn mac(&self) -> Option<Mac> {
        Some(LAN0)
    }
    fn send(&mut self, frame: &[u8]) -> Result<(), WireError> {
        self.xid = Some(dcp::decode(ether::parse(frame).unwrap().payload).unwrap().xid);
        Ok(())
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        match self.xid {
            Some(xid) => Ok(Some(Frame::new(1, "answering", dcp::identify_response(LAN0, DEV, xid, &identity()).unwrap()))),
            None => {
                std::thread::sleep(timeout);
                Ok(None)
            }
        }
    }
}

struct Stalling;
impl Wire for Stalling {
    fn name(&self) -> &str {
        "stalling"
    }
    fn mac(&self) -> Option<Mac> {
        Some(LAN0)
    }
    fn send(&mut self, _: &[u8]) -> Result<(), WireError> {
        std::thread::sleep(Duration::from_secs(2));
        Ok(())
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        std::thread::sleep(timeout);
        Ok(None)
    }
}

/// A wire that fails on its first read after it has sent a frame (a tenth of a second later).
struct FailsAfterSend {
    sent: bool,
}
impl Wire for FailsAfterSend {
    fn name(&self) -> &str {
        "fails"
    }
    fn mac(&self) -> Option<Mac> {
        Some(LAN0)
    }
    fn send(&mut self, _: &[u8]) -> Result<(), WireError> {
        self.sent = true;
        Ok(())
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        if self.sent {
            std::thread::sleep(Duration::from_millis(100));
            return Err(WireError::TooShort(2));
        }
        std::thread::sleep(timeout);
        Ok(None)
    }
}

/// A wire that panics when it is handed a frame.
struct PanicsOnSend;
impl Wire for PanicsOnSend {
    fn name(&self) -> &str {
        "panics"
    }
    fn mac(&self) -> Option<Mac> {
        Some(LAN0)
    }
    fn send(&mut self, _: &[u8]) -> Result<(), WireError> {
        panic!("the card fell out")
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        std::thread::sleep(timeout);
        Ok(None)
    }
}

/// A wire that answers the Identify it is handed with a frame its capture cut.
struct CutAnswer {
    answer: Option<Vec<u8>>,
}
impl Wire for CutAnswer {
    fn name(&self) -> &str {
        "cut"
    }
    fn mac(&self) -> Option<Mac> {
        Some(LAN0)
    }
    fn send(&mut self, frame: &[u8]) -> Result<(), WireError> {
        let pdu = dcp::decode(ether::parse(frame).unwrap().payload).unwrap();
        self.answer = Some(dcp::identify_response(LAN0, DEV, pdu.xid, &identity()).unwrap());
        Ok(())
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        match self.answer.take() {
            Some(bytes) => {
                let mut f = Frame::new(1, "cut", bytes);
                f.length += 4;
                Ok(Some(f))
            }
            None => {
                std::thread::sleep(timeout);
                Ok(None)
            }
        }
    }
}

/// A wire whose every read fails: its worker ends.
struct Broken;
impl Wire for Broken {
    fn name(&self) -> &str {
        "broken"
    }
    fn mac(&self) -> Option<Mac> {
        Some(LAN0)
    }
    fn send(&mut self, _: &[u8]) -> Result<(), WireError> {
        Ok(())
    }
    fn recv(&mut self, _: Duration) -> Result<Option<Frame>, WireError> {
        Err(WireError::TooShort(3))
    }
}

#[test]
fn asking_is_refused_where_it_cannot_be_heard_sent_or_bounded() {
    let hub = Hub::new();
    let l = Listener::start(vec![Box::new(hub.port("lan0", LAN0)), Box::new(Nameless), Box::new(Refusing), Box::new(Broken)], Options::default()).unwrap();
    let msg = |r: Result<ask::Asked, AskError>| r.map(|_| ()).unwrap_err().to_string();
    // Where no worker hears, nothing is asked.
    assert!(matches!(ask::identify(&l, "lan9", &Identify::default()), Err(AskError::Listen(ListenError::NotListening(i))) if i == "lan9"));
    // Bounded: DCP's delays, a minute's wait, a request DCP can carry.
    for delay in [0, 6401] {
        assert_eq!(msg(ask::identify(&l, "lan0", &Identify { response_delay: delay, ..Identify::default() })), format!("a response delay of {delay} is outside DCP's 1 to 6400"));
    }
    assert!(ask::identify(&l, "lan0", &Identify { response_delay: 6400, wait: Duration::ZERO, filter: Filter::Name("x".repeat(1600)) }).is_err_and(|e| matches!(e, AskError::Request(_))));
    assert_eq!(
        msg(ask::identify(&l, "lan0", &Identify { wait: Duration::from_secs(61), ..Identify::default() })),
        "a wait of 61s is past the minute an answer is waited for"
    );
    // A wire with no MAC of its own asks nothing; a wire whose worker ended neither.
    assert_eq!(msg(ask::identify(&l, "nameless", &Identify::default())), "\"nameless\" has no MAC of its own (a capture replayed): nothing is asked from it");
    assert!(l.until(Duration::from_secs(5), |s| s.stats.interfaces["broken"].ended.is_some()));
    let start = Instant::now();
    assert_eq!(msg(ask::identify(&l, "broken", &Identify::default())), "\"broken\" is heard no more (a frame of 3 bytes is shorter than an Ethernet header): nothing is asked on it");
    assert!(start.elapsed() < Duration::from_millis(500), "refused at once, not after its turn");
    // A wire that refuses the request: refused, with its reason; nothing was asked of lan0.
    let refused = msg(ask::identify(&l, "refusing", &Identify::default()));
    assert!(refused.starts_with("\"refusing\"'s wire refused the Identify: a frame of "), "{refused}");
    assert_eq!((l.interface("refusing").unwrap().refused, l.interface("lan0").unwrap().sent), (1, 0));
    // Nor is anything asked when the queue is full; and a request its worker has not sent within
    // its turn and a second is said not sent.
    let full = Listener::start(vec![Box::new(hub.port("lan1", Mac([2, 0, 0, 0, 0, 0xA1])))], Options { rate: 1, queue: 1, ..Options::default() }).unwrap();
    let frame = dcp::identify_request(LAN0, 1, 1, &Filter::All).unwrap();
    while full.send("lan1", frame.clone()).is_ok() {}
    assert!(matches!(ask::identify(&full, "lan1", &Identify::default()), Err(AskError::Listen(ListenError::QueueFull { queued: 1, .. }))));
    // A worker that ends while the request waits its turn: the request never sent, said at once.
    let ending = Listener::start(vec![Box::new(FailsAfterSend { sent: false })], Options { rate: 1, ..Options::default() }).unwrap();
    ending.send("fails", frame.clone()).unwrap();
    let start = Instant::now();
    assert_eq!(msg(ask::identify(&ending, "fails", &Identify::default())), "\"fails\" is heard no more (a frame of 2 bytes is shorter than an Ethernet header): nothing is asked on it");
    assert!(start.elapsed() < Duration::from_millis(900), "{:?}", start.elapsed());
    assert_eq!(ending.interface("fails").unwrap().unsent, 1);
    // A request handed to a wire that stalls past its turn: it may yet go, said so; one still
    // waiting behind another that stalls: withdrawn, and never sent.
    let slow = Listener::start(vec![Box::new(Stalling)], Options::default()).unwrap();
    let start = Instant::now();
    assert_eq!(msg(ask::identify(&slow, "stalling", &Identify::default())), "\"stalling\"'s wire was still taking the Identify past its turn, 1.05s: it may yet go, and no answer to it is this ask's");
    assert!(start.elapsed() >= Duration::from_millis(1050));
    let handed = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let behind = Listener::start(vec![Box::new(CountingStall { handed: Arc::clone(&handed) })], Options::default()).unwrap();
    let begun = Instant::now();
    behind.send("stalling", frame.clone()).unwrap();
    assert!(behind.until(Duration::from_secs(5), |s| s.stats.interfaces["stalling"].queued == 0));
    assert_eq!(msg(ask::identify(&behind, "stalling", &Identify::default())), "\"stalling\" did not send the Identify within its turn, 1.05s: it is withdrawn and never sent");
    // (Past the time a request left queued would have gone: two sends of two seconds each.)
    std::thread::sleep(Duration::from_millis(4500).saturating_sub(begun.elapsed()));
    let i = behind.interface("stalling").unwrap();
    assert_eq!((i.sent, i.queued, i.withdrawn, handed.load(Ordering::SeqCst)), (1, 0, 1, 1));
    // A worker that panics as it hands the request to its wire: said ended, with the panic, at once.
    let panicking = Listener::start(vec![Box::new(PanicsOnSend)], Options::default()).unwrap();
    let start = Instant::now();
    assert_eq!(msg(ask::identify(&panicking, "panics", &Identify::default())), "\"panics\" is heard no more (its worker panicked: the card fell out): nothing is asked on it");
    assert!(start.elapsed() < Duration::from_millis(900), "{:?}", start.elapsed());
    // A wait of a minute is no wait past one.
    assert_eq!(msg(ask::identify(&l, "nameless", &Identify { wait: ask::MAX_WAIT, ..Identify::default() })), "\"nameless\" has no MAC of its own (a capture replayed): nothing is asked from it");
}

/// Two bench devices of one station name behind one bridge, a plant's segment: one announces
/// itself, the other is silent (no Hello, no LLDP) and has an alias.
#[cfg(target_os = "linux")]
const SEGMENT: &str = r#"format = "veredas-bench"
version = 1

[[profinet]]
name = "announced"
interface = "d1"
station = "bomba-1"
ip = { address = "192.168.0.10", mask = "255.255.255.0" }
lldp_interval_ms = 0
seed = 1

[[profinet]]
name = "silent"
interface = "d2"
station = "bomba-1"
alias = "port-001.switch-9"
ip = { address = "192.168.0.11", mask = "255.255.255.0" }
hello = false
lldp_interval_ms = 0
seed = 2
"#;

#[cfg(target_os = "linux")]
mod bench;

#[cfg(target_os = "linux")]
#[test]
fn the_bench_s_devices_answer_an_identify_asked_through_the_listener_on_their_segment() {
    use veredas_aranha::netns;
    use veredas_aranha::observe::Probe;
    use veredas_aranha::fusion::ConflictKind;

    bench::maquinar();
    netns::isolated(
        "the_bench_s_devices_answer_an_identify_asked_through_the_listener_on_their_segment",
        &[("a0", "b0"), ("c1", "d1"), ("c2", "d2")],
        || {
            // The three MACs the test chose, set before anything opens an interface.
            let (here, announced, silent) = (Mac([0x02, 0, 0, 0, 0, 0xA0]), Mac([0x02, 0, 0, 0, 0, 0xD1]), Mac([0x02, 0, 0, 0, 0, 0xD2]));
            bench::pin(&[("a0", here), ("d1", announced), ("d2", silent)], &[("a0", "b0"), ("c1", "d1"), ("c2", "d2")]);
            assert_eq!([bench::mac_of("a0"), bench::mac_of("d1"), bench::mac_of("d2")], [here, announced, silent]);
            // One segment: a bridge holding the near ends of the devices' pairs and the far end of
            // the listener's, crossed each way before anything is heard.
            netns::ip(&["link", "add", "br0", "type", "bridge"]);
            for port in ["b0", "c1", "c2"] {
                netns::ip(&["link", "set", port, "master", "br0"]);
            }
            netns::ip(&["link", "set", "br0", "up"]);
            netns::wait_pairs(&[("a0", "d1"), ("a0", "d2")]);

            let l = Listener::start(vec![bench::link("a0")], Options::default()).unwrap();
            let run = bench::start("one-segment", SEGMENT);
            assert!(l.until(Duration::from_secs(10), |s| s.fusion.devices.contains_key(&announced)), "{:#?}", l.snapshot());
            assert!(!l.snapshot().fusion.devices.contains_key(&silent), "the silent device says nothing until asked");

            // Every device: both answer, the silent one found by asking alone.
            let all = ask::identify(&l, "a0", &Identify { filter: Filter::All, response_delay: 1, wait: Duration::from_millis(500) }).unwrap();
            let mut macs: Vec<Mac> = all.answers.iter().map(|a| a.mac).collect();
            macs.sort();
            let mut both = vec![announced, silent];
            both.sort();
            assert_eq!(macs, both, "{all:?}");
            assert_eq!((all.others, all.lost), (0, 0));
            let req = l.frame(all.request).unwrap();
            assert_eq!(ether::parse(&req.frame.bytes).unwrap().src, here);
            let now = veredas_aranha::wire::stamp(std::time::SystemTime::now());
            for a in &all.answers {
                assert!(all.sent_ns <= a.at_ns && a.at_ns <= now, "{a:?}");
                let ip = if a.mac == announced { 10 } else { 11 };
                assert_eq!(a.identity.name.as_deref(), Some("bomba-1"));
                assert_eq!(a.identity.ip.as_ref().map(|s| s.ip), Some(Ipv4Addr::new(192, 168, 0, ip)));
                assert!(a.frame > all.request);
            }
            // Fused with what was heard before: the announced device's name witnessed first by its
            // Hello and last by its answer; the silent one's by its answer alone.
            let s = l.snapshot();
            let name = &s.fusion.devices[&announced].name.candidates[0];
            let answer = |mac| all.answers.iter().find(|a| a.mac == mac).unwrap().frame;
            assert!(name.first.frame < all.request);
            assert_eq!((name.count, name.last.frame), (2, answer(announced)));
            let quiet = &s.fusion.devices[&silent];
            assert_eq!((quiet.name.candidates[0].first.frame, quiet.name.candidates[0].count), (answer(silent), 1));
            assert_eq!(quiet.heard.iter().copied().collect::<Vec<_>>(), [Probe::DcpHeard]);
            // The staged duplicate: one name on two devices of one segment, the one conflict.
            let conflicts: Vec<_> = s.conflicts.iter().map(|c| (c.kind, c.value.clone(), c.macs.clone())).collect();
            assert_eq!(conflicts, [(ConflictKind::DuplicateName, "bomba-1".to_string(), both.clone())]);

            // By name: both; by the alias: the silent one; by a name nobody holds: nobody.
            let mut asks = vec![(all.xid, Filter::All)];
            let mut by = |filter: Filter| {
                let asked = ask::identify(&l, "a0", &quick(filter.clone())).unwrap();
                asks.push((asked.xid, filter));
                let mut m: Vec<Mac> = asked.answers.iter().map(|a| a.mac).collect();
                m.sort();
                m
            };
            assert_eq!(by(Filter::Name("bomba-1".into())), both);
            assert_eq!(by(Filter::Alias("port-001.switch-9".into())), [silent]);
            assert_eq!(by(Filter::Name("bomba-2".into())), []);

            // Every DCP frame kept read by Wireshark's dissectors without complaint, the requests'
            // filters and the answers' names as asked and given.
            let (status, out, err) = bench::end(run);
            assert!(status.success(), "{out}\n{err}");
            let frames = l.frames();
            let dissected = common::dissect(&frames.iter().map(|k| k.frame.bytes.clone()).collect::<Vec<_>>());
            let mut requests = 0;
            // (Read against what the test asked and what the bench file states, not through the
            // codecs the bench and the listener share.)
            let p = "pn_dcp";
            let mut latest: Option<u32> = None;
            for (k, d) in frames.iter().zip(&dissected) {
                assert_eq!(d.complaints(), Vec::<String>::new());
                if !d.has("pn_dcp") {
                    continue;
                }
                if k.sent {
                    // Each request from this machine's MAC to DCP's group address, under its
                    // ask's transaction id and a delay of 1, its filter as asked.
                    let (xid, filter) = &asks[requests];
                    requests += 1;
                    assert_eq!((d.one("eth", "eth.src"), d.one("eth", "eth.dst")), (here.to_string(), "01:0e:cf:00:00:00".to_string()));
                    assert_eq!((d.one(p, "pn_dcp.xid"), d.one(p, "pn_dcp.response_delay")), (xid.to_string(), "1".to_string()));
                    // An Identify under its request's frame id, its one filter block as asked.
                    assert_eq!((d.one("pn_rt", "pn_rt.frame_id"), d.one(p, "pn_dcp.service_id")), ("65278".to_string(), "5".to_string()));
                    match filter {
                        Filter::All => assert_eq!(d.values(p, "pn_dcp.option"), ["255"]),
                        Filter::Name(n) => {
                            assert_eq!(&d.one(p, "pn_dcp.suboption_device_nameofstation"), n);
                            assert_eq!(d.values(p, "pn_dcp.option"), ["2"]);
                        }
                        Filter::Alias(a) => {
                            assert_eq!(&d.one(p, "pn_dcp.suboption_device_aliasname"), a);
                            assert_eq!(d.values(p, "pn_dcp.option"), ["2"]);
                        }
                    }
                    latest = Some(*xid);
                } else {
                    // A Hello under the Hello's frame id; an answer under the answer's, to this
                    // machine's MAC under the latest request's id.
                    match d.one(p, "pn_dcp.service_id").as_str() {
                        "6" => assert_eq!(d.one("pn_rt", "pn_rt.frame_id"), "65276"),
                        "5" => {
                            assert_eq!((d.one("pn_rt", "pn_rt.frame_id"), d.one("eth", "eth.dst")), ("65279".to_string(), here.to_string()));
                            assert_eq!(Some(d.one(p, "pn_dcp.xid")), latest.map(|x| x.to_string()));
                        }
                        other => panic!("DCP service {other} from the bench"),
                    }
                    // Each answer as the bench file states it.
                    let src = d.one("eth", "eth.src");
                    let ip = if src == announced.to_string() { "192.168.0.10" } else { assert_eq!(src, silent.to_string()); "192.168.0.11" };
                    assert_eq!(d.one(p, "pn_dcp.suboption_device_nameofstation"), "bomba-1");
                    assert_eq!((d.one(p, "pn_dcp.suboption_ip_ip"), d.one(p, "pn_dcp.suboption_ip_subnetmask")), (ip.to_string(), "255.255.255.0".to_string()));
                }
            }
            assert_eq!(requests, 4);
        },
    );
}

#[test]
fn askers_at_once_each_take_their_turn_and_their_answers() {
    let hub = Hub::new();
    let _dev = Device::start(hub.port("dev", DEV), Manner::default());
    let l = Arc::new(Listener::start(vec![Box::new(hub.port("lan0", LAN0))], Options { rate: 1, ..Options::default() }).unwrap());
    let askers: Vec<_> = (0..5)
        .map(|_| {
            let l = Arc::clone(&l);
            std::thread::spawn(move || ask::identify(&l, "lan0", &quick(Filter::All)))
        })
        .collect();
    let asked: Vec<ask::Asked> = askers.into_iter().map(|h| h.join().unwrap().unwrap()).collect();
    let xids: std::collections::BTreeSet<u32> = asked.iter().map(|a| a.xid).collect();
    assert_eq!(xids.len(), 5);
    assert!(asked.iter().all(|a| a.answers.len() == 1), "{asked:?}");
    let i = l.interface("lan0").unwrap();
    assert_eq!((i.sent, i.withdrawn, i.queued), (5, 0, 0));
}

#[test]
fn frames_let_go_while_answers_flood_are_counted_at_the_moment_they_are_read() {
    // A device that streams answers to the request under its id through the asker's read, the
    // listener keeping 64 frames: the first answer read is the first frame kept after the request
    // and the lost, so none was let go between the count and the read.
    let hub = Hub::new();
    let mut port = hub.port("dev", DEV);
    let stop = Arc::new(AtomicBool::new(false));
    let halt = Arc::clone(&stop);
    let device = std::thread::spawn(move || {
        while !halt.load(Ordering::SeqCst) {
            let Ok(Some(f)) = port.recv(Duration::from_millis(20)) else { continue };
            let Ok(e) = ether::parse(&f.bytes) else { continue };
            let Ok(pdu) = dcp::decode(e.payload) else { continue };
            // Answers streamed under the request's id for 300 ms, through the asker's read.
            if pdu.service == Service::Identify && !pdu.service_type.is_response() {
                let answer = dcp::identify_response(e.src, DEV, pdu.xid, &identity()).unwrap();
                let until = Instant::now() + Duration::from_millis(300);
                while Instant::now() < until {
                    port.send(&answer).unwrap();
                    std::thread::sleep(Duration::from_micros(50));
                }
            }
        }
    });
    let l = Listener::start(vec![Box::new(hub.port("lan0", LAN0))], Options { max_frames: 64, ..Options::default() }).unwrap();
    for _ in 0..10 {
        let asked = ask::identify(&l, "lan0", &Identify { filter: Filter::All, response_delay: 1, wait: Duration::from_millis(150) }).unwrap();
        let first = asked.answers.first().expect("answers kept");
        assert_eq!(first.frame, asked.request + 1 + asked.lost, "{} lost", asked.lost);
        assert!(asked.lost > 0, "the flood passed the bound");
        // (The stream of this ask's answers ends before the next asks.)
        std::thread::sleep(Duration::from_millis(400));
    }
    stop.store(true, Ordering::SeqCst);
    device.join().unwrap();
}

#[test]
fn an_answer_a_capture_cut_is_not_counted() {
    let l = Listener::start(vec![Box::new(CutAnswer { answer: None })], Options::default()).unwrap();
    let asked = ask::identify(&l, "cut", &quick(Filter::All)).unwrap();
    assert_eq!((asked.answers.len(), asked.others), (0, 0));
    assert!(l.frame(asked.request + 1).unwrap().frame.is_cut());
}

#[test]
fn a_request_whose_worker_ends_as_its_turn_runs_out_is_never_said_still_being_taken() {
    // A filler of 200 000 kept frames, so a reader of every kept frame holds the listener a while;
    // six wires that panic 2 ms before an asker's turn runs out, a frame of their own in hand.
    let filler: Vec<Frame> = (0..200_000u64).map(|i| Frame::new(i, "filler", vec![0; 60])).collect();
    let begun: Vec<_> = (0..6).map(|_| Arc::new(std::sync::Mutex::new(None))).collect();
    let mut wires: Vec<Box<dyn Wire>> = vec![Box::new(Replay::new("filler", filler))];
    for (k, b) in begun.iter().enumerate() {
        wires.push(Box::new(PanicsLate { name: format!("race{k}"), began: Arc::clone(b), panic_after: Duration::from_millis(1048) }));
    }
    let l = Arc::new(Listener::start(wires, Options { max_frames: 300_000, ..Options::default() }).unwrap());
    assert!(l.until(Duration::from_secs(60), |s| s.stats.interfaces["filler"].ended.is_some()));
    let stop = Arc::new(AtomicBool::new(false));
    let reader = {
        let (l, stop) = (Arc::clone(&l), Arc::clone(&stop));
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let _ = l.frames();
            }
        })
    };
    let frame = dcp::identify_request(LAN0, 1, 1, &Filter::All).unwrap();
    for (k, b) in begun.iter().enumerate() {
        let iface = format!("race{k}");
        l.send(&iface, frame.clone()).unwrap();
        let wait = Instant::now();
        while b.lock().unwrap().is_none() {
            assert!(wait.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(1));
        }
        // Withdrawn (never sent) or ended (never sent): never said still being taken by its wire.
        match ask::identify(&l, &iface, &Identify::default()) {
            Err(AskError::Ended { reason, .. }) => assert_eq!(reason, "its worker panicked: the card fell out"),
            Err(AskError::NotSent { .. }) => {}
            other => panic!("{iface}: {other:?}"),
        }
        let i = l.interface(&iface).unwrap();
        assert_eq!(i.sent, 0, "{iface}");
    }
    stop.store(true, Ordering::SeqCst);
    reader.join().unwrap();
}

#[test]
fn frames_let_go_while_a_wire_floods_without_a_pause_are_counted_at_the_moment_they_are_read() {
    let l = Listener::start(vec![Box::new(Answering { xid: None })], Options { max_frames: 64, ..Options::default() }).unwrap();
    for _ in 0..20 {
        let asked = ask::identify(&l, "answering", &Identify { filter: Filter::All, response_delay: 1, wait: Duration::from_millis(20) }).unwrap();
        let first = asked.answers.first().expect("answers kept");
        assert_eq!(first.frame, asked.request + 1 + asked.lost, "{} lost", asked.lost);
        assert!(asked.lost > 0);
    }
}
