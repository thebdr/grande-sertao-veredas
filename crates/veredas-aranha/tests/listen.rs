//! The listener (contract: the listener clause): one worker per wire, every frame kept and decoded,
//! its observations fused but for this machine's own, counted by class, damage kept; a paced and
//! bounded sender only where it listens; on the memory hub and a capture replayed.

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use veredas_aranha::listen::{ListenError, Listener, Options, Outcome};
use veredas_aranha::observe::{self, Class, Probe};
use veredas_aranha::profinet::dcp::{self, DeviceIdentity, IpSuite};
use veredas_aranha::profinet::ether;
use veredas_aranha::profinet::lldp::{self, ChassisId, LldpPdu, PortId};
use veredas_aranha::profinet::mac::Mac;
use veredas_aranha::profinet::rt::{CyclicFrame, DataStatus};
use veredas_aranha::wire::hub::Hub;
use veredas_aranha::wire::pcap::Replay;
use veredas_aranha::wire::{Frame, Wire, WireError};

const DEV: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]);
const PORT: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0x02]);
const LAN0: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0xA0]);
const LAN1: Mac = Mac([0x02, 0x00, 0x00, 0x00, 0x00, 0xA1]);

fn identity() -> DeviceIdentity {
    DeviceIdentity {
        name: Some("io-device-1".into()),
        ip: Some(IpSuite { ip: Ipv4Addr::new(192, 168, 0, 10), mask: Ipv4Addr::new(255, 255, 255, 0), gateway: Ipv4Addr::UNSPECIFIED }),
        vendor_id: Some(0x1234),
        device_id: Some(0x0203),
        ..Default::default()
    }
}

/// What a PROFINET device says on its own: a Hello, its port's LLDP, an ARP, a cyclic frame.
fn device_frames() -> Vec<Vec<u8>> {
    let mut pdu = LldpPdu::new(ChassisId::Local { id: "io-device-1".into() }, PortId::Local { id: "port-001.io-device-1".into() }, 20);
    pdu.pn.chassis_mac = Some(DEV);
    vec![
        dcp::hello(DEV, 1, &identity()).unwrap(),
        lldp::frame(PORT, &pdu).unwrap(),
        observe::arp_frame(DEV, Ipv4Addr::new(192, 168, 0, 10), Mac::BROADCAST, Ipv4Addr::new(192, 168, 0, 1), false).unwrap(),
        CyclicFrame { frame_id: 0x8001, data: vec![0; 40], cycle_counter: 1, data_status: DataStatus::GOOD, transfer_status: 0 }.frame(LAN0, DEV, None).unwrap(),
    ]
}

const WAIT: Duration = Duration::from_secs(10);

#[test]
fn a_listener_keeps_every_frame_it_hears_and_fuses_what_it_says() {
    let hub = Hub::new();
    let mut dev = hub.port("dev", DEV);
    let listener = Listener::start(vec![Box::new(hub.port("lan0", LAN0))], Options::default()).unwrap();
    hub.set_now(1_000);
    let sent = device_frames();
    for f in &sent {
        dev.send(f).unwrap();
    }
    assert!(listener.until(WAIT, |s| s.stats.heard == 4), "{:?}", listener.snapshot().stats);
    let s = listener.snapshot();
    // Every frame kept as it was heard, in order, its interface and time with it.
    assert_eq!(s.kept, (0, 4));
    for (i, f) in sent.iter().enumerate() {
        let k = listener.frame(i as u64).unwrap();
        assert_eq!((k.frame.bytes.as_slice(), k.frame.iface.as_str(), k.frame.at_ns, k.sent), (f.as_slice(), "lan0", 1_000, false));
    }
    assert_eq!(listener.frame(4), None);
    // Counted by class; three observations fused into one device, the cyclic frame said nothing.
    let classes: Vec<(Class, u64)> = s.stats.by_class.iter().map(|(c, n)| (*c, *n)).collect();
    assert_eq!(classes, [(Class::Dcp, 1), (Class::Cyclic, 1), (Class::Lldp, 1), (Class::Arp, 1)]);
    assert_eq!((s.stats.heard, s.stats.observations, s.stats.own, s.stats.damaged_frames), (4, 3, 0, 0));
    assert_eq!((s.stats.first_ns, s.stats.last_ns), (Some(1_000), Some(1_000)));
    let lan0 = &s.stats.interfaces["lan0"];
    assert_eq!((lan0.mac, lan0.heard, lan0.sent, lan0.queued, lan0.ended.clone()), (Some(LAN0), 4, 0, 0, None));
    assert_eq!(s.fusion.devices.len(), 1);
    let d = &s.fusion.devices[&DEV];
    assert_eq!((d.label(), d.frames), ("io-device-1".to_string(), 3));
    assert_eq!(d.heard.iter().copied().collect::<Vec<_>>(), [Probe::DcpHeard, Probe::LldpHeard, Probe::ArpHeard]);
    // Each value's witness names the kept frame it came from: decoding it again gives the same.
    let w = &d.name.candidates[0].last;
    let again = observe::decode(&listener.frame(w.frame).unwrap().frame, w.frame);
    assert_eq!(again.observations[0].witness, *w);
    assert!(s.conflicts.is_empty());
    let end = listener.stop();
    assert_eq!(end.stats.heard, 4);
}

#[test]
fn a_capture_replayed_gives_the_same_devices_every_time_and_ends() {
    let frames: Vec<Frame> = device_frames().into_iter().enumerate().map(|(i, b)| Frame::new(1_000 * (i as u64 + 1), "cap", b)).collect();
    let run = || {
        let listener = Listener::start(vec![Box::new(Replay::new("cap", frames.clone()))], Options::default()).unwrap();
        assert!(listener.until(WAIT, |s| s.stats.interfaces["cap"].ended.is_some()));
        listener.stop()
    };
    let (a, b) = (run(), run());
    assert_eq!(a.fusion, b.fusion);
    assert_eq!(a.stats.interfaces["cap"].ended.as_deref(), Some("its capture is read to the end"));
    assert_eq!((a.stats.heard, a.stats.first_ns, a.stats.last_ns), (4, Some(1_000), Some(4_000)));
    // A capture has no MAC of its own: nothing is left out as this machine's.
    assert_eq!((a.stats.interfaces["cap"].mac, a.stats.own), (None, 0));
}

#[test]
fn what_is_about_this_machine_is_left_out_and_counted() {
    // Two of this machine's interfaces on one segment: what one sends the other hears, and it is
    // about this machine, so it is left out and counted; the device hears it, and it is kept as sent.
    let hub = Hub::new();
    let mut dev = hub.port("dev", DEV);
    let listener = Listener::start(vec![Box::new(hub.port("lan0", LAN0)), Box::new(hub.port("lan1", LAN1))], Options::default()).unwrap();
    let ask = dcp::identify_request(LAN0, 9, 1, &dcp::Filter::All).unwrap();
    listener.send("lan0", ask.clone()).unwrap();
    assert!(listener.until(WAIT, |s| s.stats.interfaces["lan1"].heard == 1 && s.stats.interfaces["lan0"].sent == 1));
    assert_eq!(dev.recv(WAIT).unwrap().map(|f| f.bytes), Some(ask.clone()));
    let s = listener.snapshot();
    assert_eq!((s.stats.own, s.stats.observations, s.fusion.devices.len()), (1, 0, 0));
    let kept = listener.frames();
    assert_eq!(kept.iter().filter(|k| k.sent).map(|k| (k.frame.iface.as_str(), k.frame.bytes.clone())).collect::<Vec<_>>(), [("lan0", ask)]);
    // The device's answer, heard on both interfaces, is fused once per hearing into one device.
    dev.send(&dcp::identify_response(LAN0, DEV, 9, &identity()).unwrap()).unwrap();
    assert!(listener.until(WAIT, |s| s.stats.observations == 2));
    let s = listener.snapshot();
    assert_eq!(s.fusion.devices[&DEV].heard_on.iter().cloned().collect::<Vec<_>>(), ["lan0", "lan1"]);
}

#[test]
fn sending_is_paced_bounded_and_only_where_it_listens() {
    let hub = Hub::new();
    let mut dev = hub.port("dev", DEV);
    let f = |i: u8| ether::build(DEV, LAN0, 0x88B5, &[i; 46]).unwrap();
    // Twenty frames a second: ten sent take at least nine gaps of 50 ms, heard in order.
    let listener = Listener::start(vec![Box::new(hub.port("lan0", LAN0))], Options { rate: 20, ..Options::default() }).unwrap();
    for i in 0..10 {
        listener.send("lan0", f(i)).unwrap();
    }
    let mut times = Vec::new();
    for i in 0..10 {
        let got = dev.recv(WAIT).unwrap().unwrap();
        times.push(Instant::now());
        assert_eq!(got.bytes, f(i));
    }
    let span = times[9] - times[0];
    assert!(span >= Duration::from_millis(9 * 50 - 20) && span < Duration::from_secs(5), "{span:?}");
    assert!(listener.until(WAIT, |s| s.stats.interfaces["lan0"].sent == 10));
    drop(listener);
    // Bounded: at one frame a second, the first sent at once, three queued behind it, a fourth
    // refused; nowhere it does not listen; of a length no wire carries.
    let listener = Listener::start(vec![Box::new(hub.port("lan0", LAN0))], Options { rate: 1, queue: 3, ..Options::default() }).unwrap();
    listener.send("lan0", f(0)).unwrap();
    assert!(listener.until(WAIT, |s| s.stats.interfaces["lan0"].sent == 1));
    for i in 1..=3 {
        listener.send("lan0", f(i)).unwrap();
    }
    assert_eq!(listener.snapshot().stats.interfaces["lan0"].queued, 3);
    let err = listener.send("lan0", f(4)).unwrap_err();
    assert!(matches!(&err, ListenError::QueueFull { iface, queued: 3 } if iface == "lan0"), "{err}");
    assert_eq!(err.to_string(), "\"lan0\" has 3 frames queued, its bound: the frame is refused");
    let err = listener.send("lan9", f(5)).unwrap_err();
    assert!(matches!(&err, ListenError::NotListening(i) if i == "lan9"));
    assert_eq!(err.to_string(), "no wire is heard on \"lan9\": nothing is sent where the listener does not listen");
    assert!(matches!(listener.send("lan0", vec![0; 13]), Err(ListenError::Frame(WireError::TooShort(13)))));
    assert!(matches!(listener.send("lan0", vec![0; 65_537]), Err(ListenError::Frame(WireError::TooLong(65_537)))));
    // Two wires of one name, a rate of 0 and no frames kept are refused.
    let two = Listener::start(vec![Box::new(hub.port("x", LAN0)), Box::new(hub.port("x", LAN1))], Options::default());
    assert!(matches!(two, Err(ListenError::TwoWires(n)) if n == "x"));
    assert!(matches!(Listener::start(vec![], Options { rate: 0, ..Options::default() }), Err(ListenError::Options)));
    let fast = Listener::start(vec![], Options { rate: 1001, ..Options::default() }).map(|_| ()).unwrap_err();
    assert_eq!(fast.to_string(), "a listener sends one to a thousand frames a second and keeps at least one frame");
    assert!(Listener::start(vec![], Options { rate: 1000, ..Options::default() }).is_ok());
    assert!(matches!(Listener::start(vec![], Options { max_frames: 0, ..Options::default() }), Err(ListenError::Options)));
}

#[test]
fn the_kept_frames_are_bounded_and_damage_is_kept_with_its_frame() {
    let hub = Hub::new();
    let mut dev = hub.port("dev", DEV);
    let listener = Listener::start(vec![Box::new(hub.port("lan0", LAN0))], Options { max_frames: 5, ..Options::default() }).unwrap();
    // Sixty DCP frames of a service DCP does not define: each damaged, the last fifty kept.
    let bad = |i: u8| {
        let mut p = vec![0xFE, 0xFE, 7, 0, 0, 0, 0, i, 0, 0, 0, 0];
        p.resize(46, 0);
        ether::build(LAN0, DEV, ether::ETHERTYPE_PROFINET, &p).unwrap()
    };
    for i in 0..60 {
        dev.send(&bad(i)).unwrap();
    }
    assert!(listener.until(WAIT, |s| s.stats.heard == 60));
    let s = listener.snapshot();
    // Five kept, the oldest 55 let go; their indices no longer kept.
    assert_eq!((s.kept, s.stats.let_go, listener.frames().len()), ((55, 60), 55, 5));
    assert_eq!(listener.frame(54), None);
    assert_eq!(listener.frame(59).unwrap().frame.bytes, bad(59));
    assert_eq!((s.stats.damaged_frames, s.stats.damage.len()), (60, 50));
    assert_eq!(s.stats.damage.front().unwrap(), "#10 lan0: DCP service 7 is none of DCP's (3 Get, 4 Set, 5 Identify, 6 Hello)");
    assert_eq!(s.stats.damage.back().unwrap(), "#59 lan0: DCP service 7 is none of DCP's (3 Get, 4 Set, 5 Identify, 6 Hello)");
    // A wire's error ends its worker, named; the other goes on.
    struct Broken;
    impl Wire for Broken {
        fn name(&self) -> &str {
            "broken"
        }
        fn mac(&self) -> Option<Mac> {
            None
        }
        fn send(&mut self, _: &[u8]) -> Result<(), WireError> {
            Err(WireError::TooShort(0))
        }
        fn recv(&mut self, _: Duration) -> Result<Option<Frame>, WireError> {
            Err(WireError::TooLong(1))
        }
    }
    let listener = Listener::start(vec![Box::new(Broken), Box::new(hub.port("lan0", LAN0))], Options::default()).unwrap();
    assert!(listener.until(WAIT, |s| s.stats.interfaces["broken"].ended.is_some()));
    dev.send(&dcp::hello(DEV, 1, &identity()).unwrap()).unwrap();
    assert!(listener.until(WAIT, |s| s.stats.interfaces["lan0"].heard == 1));
    assert_eq!(listener.snapshot().stats.interfaces["broken"].ended.as_deref(), Some("a frame of 1 bytes is past the 65536 a wire carries"));
    // A wire that refuses a frame: counted, and the reason kept with the damage; nothing kept as sent.
    struct Refusing;
    impl Wire for Refusing {
        fn name(&self) -> &str {
            "refusing"
        }
        fn mac(&self) -> Option<Mac> {
            None
        }
        fn send(&mut self, frame: &[u8]) -> Result<(), WireError> {
            Err(WireError::TooLong(frame.len() * 1000))
        }
        fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
            std::thread::sleep(timeout);
            Ok(None)
        }
    }
    let listener = Listener::start(vec![Box::new(Refusing)], Options::default()).unwrap();
    let told = listener.send_tracked("refusing", vec![0; 60]).unwrap().told;
    assert_eq!(told.recv_timeout(WAIT), Ok(Outcome::Refused("a frame of 60000 bytes is past the 65536 a wire carries".into())));
    assert_eq!(told.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected), "told once");
    assert!(listener.until(WAIT, |s| s.stats.interfaces["refusing"].refused == 1));
    let s = listener.snapshot();
    assert_eq!((s.stats.interfaces["refusing"].sent, s.stats.damage.back().cloned()), (0, Some("refusing: send refused: a frame of 60000 bytes is past the 65536 a wire carries".to_string())));
    assert!(listener.frames().is_empty());
}

#[test]
fn stopping_joins_every_worker_and_nothing_is_heard_after() {
    let hub = Hub::new();
    let mut dev = hub.port("dev", DEV);
    let listener = Listener::start(vec![Box::new(hub.port("lan0", LAN0)), Box::new(hub.port("lan1", LAN1))], Options::default()).unwrap();
    dev.send(&dcp::hello(DEV, 1, &identity()).unwrap()).unwrap();
    assert!(listener.until(WAIT, |s| s.stats.heard == 2));
    let started = Instant::now();
    let end = listener.stop();
    assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
    assert_eq!(end.stats.heard, 2);
    // Its ports left the segment with it: a frame sent now reaches no one.
    assert_eq!(hub.ports(), 1);
    dev.send(&dcp::hello(DEV, 2, &identity()).unwrap()).unwrap();
    // A listener dropped without being stopped joins its workers too.
    let dropped = Listener::start(vec![Box::new(hub.port("lan0", LAN0))], Options::default()).unwrap();
    let started = Instant::now();
    drop(dropped);
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(hub.ports(), 1);
}

/// A wire with a MAC that records when each frame was handed to it, and hears nothing.
struct Recording {
    sends: std::sync::Arc<std::sync::Mutex<Vec<Instant>>>,
}
impl Wire for Recording {
    fn name(&self) -> &str {
        "rec"
    }
    fn mac(&self) -> Option<Mac> {
        Some(LAN0)
    }
    fn send(&mut self, _: &[u8]) -> Result<(), WireError> {
        self.sends.lock().unwrap().push(Instant::now());
        Ok(())
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        std::thread::sleep(timeout);
        Ok(None)
    }
}

/// A wire that fails on every read once it has sent a frame, counting its reads.
struct FailsAfterSend {
    sent: bool,
    reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl Wire for FailsAfterSend {
    fn name(&self) -> &str {
        "fails"
    }
    fn mac(&self) -> Option<Mac> {
        Some(LAN1)
    }
    fn send(&mut self, _: &[u8]) -> Result<(), WireError> {
        self.sent = true;
        Ok(())
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        if self.sent {
            // (A tenth of a second first, so every frame the test queues is queued.)
            std::thread::sleep(Duration::from_millis(100));
            self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Err(WireError::TooShort(1));
        }
        std::thread::sleep(timeout);
        Ok(None)
    }
}

#[test]
fn the_defaults_and_the_pace_are_what_the_listener_promises() {
    assert_eq!(Options::default(), Options { max_frames: 200_000, rate: 20, queue: 100, poll: Duration::from_millis(20) });
    // Measured where the wire is handed each frame: the first within the poll (20 ms, at most half
    // a gap), then one a gap, no slower over twenty gaps; and after a quiet second, no burst.
    for (rate, n, idle) in [(2u32, 4usize, false), (20, 21, false), (100, 21, false), (20, 10, true)] {
        let sends = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let l = Listener::start(vec![Box::new(Recording { sends: std::sync::Arc::clone(&sends) })], Options { rate, ..Options::default() }).unwrap();
        if idle {
            l.send("rec", vec![0; 60]).unwrap();
            std::thread::sleep(Duration::from_secs(1));
            sends.lock().unwrap().clear();
        }
        let gap = Duration::from_secs(1) / rate;
        let poll = Duration::from_millis(20).min(gap / 2);
        let start = Instant::now();
        for _ in 0..n {
            l.send("rec", vec![0; 60]).unwrap();
        }
        assert!(l.until(WAIT, |s| s.stats.interfaces["rec"].queued == 0 && sends.lock().unwrap().len() == n));
        let at = sends.lock().unwrap().clone();
        assert!(at[0] - start <= poll + Duration::from_millis(15), "at {rate} a second the first went after {:?}", at[0] - start);
        for w in at.windows(2) {
            assert!(w[1] - w[0] >= gap - Duration::from_millis(1), "at {rate} a second a gap of {:?}", w[1] - w[0]);
        }
        let span = at[n - 1] - at[0];
        assert!(span <= gap * (n as u32 - 1) + Duration::from_millis(100), "at {rate} a second {n} frames spanned {span:?}");
    }
    // A frame handed over while the worker waits on its wire goes within half a gap, not a whole
    // poll: at a thousand a second (a poll of half a millisecond), eleven single frames handed
    // over at different moments of the worker's wait, their median delay a few milliseconds at
    // most (an unbounded poll of 20 ms gives about ten).
    let sends = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let l = Listener::start(vec![Box::new(Recording { sends: std::sync::Arc::clone(&sends) })], Options { rate: 1000, ..Options::default() }).unwrap();
    let mut delays = Vec::new();
    for k in 0..11u64 {
        std::thread::sleep(Duration::from_millis(3 + k % 7 * 2));
        let before = sends.lock().unwrap().len();
        let handed = Instant::now();
        l.send("rec", vec![0; 60]).unwrap();
        assert!(l.until(WAIT, |_| sends.lock().unwrap().len() > before));
        delays.push(sends.lock().unwrap()[before] - handed);
    }
    delays.sort();
    assert!(delays[5] <= Duration::from_millis(5), "a thousand a second, the delays {delays:?}");
    // Frames the wire refuses keep the pace too.
    let handed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let l = Listener::start(vec![Box::new(RefusingRecorder { handed: std::sync::Arc::clone(&handed) })], Options::default()).unwrap();
    for _ in 0..3 {
        l.send("refuses", vec![0; 60]).unwrap();
    }
    assert!(l.until(WAIT, |s| s.stats.interfaces["refuses"].refused == 3));
    let at = handed.lock().unwrap().clone();
    assert!(at.windows(2).all(|w| w[1] - w[0] >= Duration::from_millis(49)), "{at:?}");
}

/// A wire with a MAC that records when each frame was handed to it, and refuses it.
struct RefusingRecorder {
    handed: std::sync::Arc<std::sync::Mutex<Vec<Instant>>>,
}
impl Wire for RefusingRecorder {
    fn name(&self) -> &str {
        "refuses"
    }
    fn mac(&self) -> Option<Mac> {
        Some(LAN0)
    }
    fn send(&mut self, frame: &[u8]) -> Result<(), WireError> {
        self.handed.lock().unwrap().push(Instant::now());
        Err(WireError::TooLong(frame.len() * 1000))
    }
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        std::thread::sleep(timeout);
        Ok(None)
    }
}

/// A wire that panics on its first read after it has sent a frame (a tenth of a second later).
struct Panicking {
    sent: bool,
}
impl Wire for Panicking {
    fn name(&self) -> &str {
        "panics"
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
            panic!("the wire broke");
        }
        std::thread::sleep(timeout);
        Ok(None)
    }
}

#[test]
fn a_worker_that_panics_ends_with_its_reason_and_a_wire_name_may_hold_a_nul() {
    let l = Listener::start(vec![Box::new(Panicking { sent: false })], Options { rate: 1, ..Options::default() }).unwrap();
    let first = l.send_tracked("panics", vec![0; 60]).unwrap().told;
    let second = l.send_tracked("panics", vec![1; 60]).unwrap().told;
    assert!(matches!(first.recv_timeout(WAIT), Ok(Outcome::Sent { .. })));
    let reason = "its worker panicked: the wire broke".to_string();
    assert_eq!(second.recv_timeout(WAIT), Ok(Outcome::Unsent(reason.clone())));
    assert_eq!(second.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected), "told once");
    let i = l.interface("panics").unwrap();
    assert_eq!((i.sent, i.unsent, i.queued, i.ended.clone()), (1, 1, 0, Some(reason.clone())));
    assert!(matches!(l.send("panics", vec![0; 60]), Err(ListenError::Ended { reason: r, .. }) if r == reason));
    // A wire named with a NUL: its worker starts, and every worker stops with the listener.
    let hub = Hub::new();
    let l = Listener::start(vec![Box::new(hub.port("lan0", LAN0)), Box::new(Replay::new("cap\0ture", Vec::new()))], Options::default()).unwrap();
    assert!(l.until(WAIT, |s| s.stats.interfaces["cap\0ture"].ended.is_some()));
    l.stop();
    assert_eq!(hub.ports(), 0);
}

#[test]
fn an_ended_worker_takes_nothing_more_and_says_what_it_never_sent() {
    // A capture read to its end: nothing is queued on it any more.
    let l = Listener::start(vec![Box::new(Replay::new("cap", Vec::new()))], Options::default()).unwrap();
    assert!(l.until(WAIT, |s| s.stats.interfaces["cap"].ended.is_some()));
    let refused = l.send("cap", vec![0; 60]).unwrap_err();
    assert!(matches!(&refused, ListenError::Ended { iface, reason } if iface == "cap" && reason == "its capture is read to the end"));
    assert_eq!(refused.to_string(), "\"cap\" is heard no more (its capture is read to the end): nothing is sent on it");
    assert_eq!(l.interface("cap").unwrap().queued, 0);
    // A wire that fails once it has sent its first frame, four more queued at one a second: the
    // four are said unsent, the queue empty, its worker read once after the failure and no more.
    let reads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let l = Listener::start(vec![Box::new(FailsAfterSend { sent: false, reads: std::sync::Arc::clone(&reads) })], Options { rate: 1, ..Options::default() }).unwrap();
    for _ in 0..4 {
        l.send("fails", vec![0; 60]).unwrap();
    }
    let told = l.send_tracked("fails", vec![1; 60]).unwrap().told;
    assert!(l.until(WAIT, |s| s.stats.interfaces["fails"].ended.is_some()));
    let reason = "a frame of 1 bytes is shorter than an Ethernet header".to_string();
    assert_eq!(told.recv_timeout(WAIT), Ok(Outcome::Unsent(reason.clone())));
    assert_eq!(told.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected), "told once");
    let i = l.interface("fails").unwrap();
    assert_eq!((i.sent, i.queued, i.unsent, i.refused, i.ended), (1, 0, 4, 0, Some(reason)));
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(reads.load(std::sync::atomic::Ordering::SeqCst), 1);
    // Stopping says unsent what was still queued, its interface not ended.
    let hub = Hub::new();
    let l = Listener::start(vec![Box::new(hub.port("lan0", LAN0))], Options { rate: 1, ..Options::default() }).unwrap();
    l.send("lan0", vec![0; 60]).unwrap();
    assert!(l.until(WAIT, |s| s.stats.interfaces["lan0"].sent == 1));
    let told = l.send_tracked("lan0", vec![0; 60]).unwrap().told;
    l.send("lan0", vec![0; 60]).unwrap();
    let s = l.stop();
    let i = &s.stats.interfaces["lan0"];
    assert_eq!((i.sent, i.queued, i.unsent, i.ended.clone()), (1, 0, 2, None));
    assert_eq!(told.recv_timeout(WAIT), Ok(Outcome::Unsent("the listener stopped".into())));
    assert_eq!(told.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected), "told once");
}

#[test]
fn every_frame_is_the_interface_s_that_heard_it_and_its_account_says_so() {
    // A capture whose frames were recorded as "eth0", replayed as "plant.pcap", stamps out of
    // order, one frame with two damages.
    let mut frames: Vec<Frame> = device_frames().into_iter().enumerate().map(|(i, b)| Frame::new([300, 100, 200, 250][i], "eth0", b)).collect();
    let mut twice = dcp::hello(DEV, 1, &identity()).unwrap();
    let len = u16::from_be_bytes([twice[24], twice[25]]) as usize;
    twice.truncate(26 + len);
    twice.extend_from_slice(&[0xAB, 0xCD, 0xEF]);
    twice[24..26].copy_from_slice(&((len + 3) as u16).to_be_bytes());
    twice[6] |= 0x01;
    frames.push(Frame::new(150, "eth0", twice));
    let l = Listener::start(vec![Box::new(Replay::new("plant.pcap", frames))], Options::default()).unwrap();
    assert!(l.until(WAIT, |s| s.stats.interfaces["plant.pcap"].ended.is_some()));
    let s = l.snapshot();
    assert_eq!(s.stats.interfaces.keys().cloned().collect::<Vec<_>>(), ["plant.pcap"]);
    assert_eq!((s.stats.interfaces["plant.pcap"].heard, s.stats.heard), (5, 5));
    assert!(l.frames().iter().all(|k| k.frame.iface == "plant.pcap"));
    assert!(s.fusion.devices[&DEV].heard_on.iter().eq(["plant.pcap"]));
    assert_eq!((s.stats.first_ns, s.stats.last_ns), (Some(100), Some(300)), "the earliest and the latest stamps");
    assert_eq!(s.stats.damaged_frames, 1);
    assert_eq!(s.stats.damage.iter().filter(|d| d.starts_with("#4 plant.pcap: ")).count(), 2, "{:?}", s.stats.damage);
    // A frame sent: its account sent once, nothing heard, nothing queued, the frame stamped when
    // it went; frames of 14 and 65 536 bytes go.
    let hub = Hub::new();
    let mut dev = hub.port("dev", DEV);
    let l = Listener::start(vec![Box::new(hub.port("lan0", LAN0))], Options::default()).unwrap();
    let before = veredas_aranha::wire::stamp(std::time::SystemTime::now());
    let told: Vec<_> = [14, 65_536].map(|n| l.send_tracked("lan0", vec![0; n]).unwrap().told).into_iter().collect();
    assert!(l.until(WAIT, |s| s.stats.interfaces["lan0"].sent == 2));
    let after = veredas_aranha::wire::stamp(std::time::SystemTime::now());
    // Each told where its frame was kept, once.
    for (rx, n) in told.iter().zip([14, 65_536]) {
        let Ok(Outcome::Sent { index, at_ns }) = rx.recv_timeout(WAIT) else { panic!("{n} bytes: not told sent") };
        let kept = l.frame(index).unwrap();
        assert_eq!((kept.sent, kept.frame.iface.as_str(), kept.frame.at_ns, kept.frame.bytes.len()), (true, "lan0", at_ns, n));
        assert_eq!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected), "told once");
    }
    let s = l.snapshot();
    let i = &s.stats.interfaces["lan0"];
    assert_eq!((i.heard, i.sent, i.queued, s.stats.heard), (0, 2, 0, 0));
    assert_eq!((s.stats.first_ns, s.stats.last_ns), (None, None), "the stamps heard, none");
    assert!(l.frames().iter().all(|k| k.sent && (before..=after).contains(&k.frame.at_ns)));
    assert_eq!([dev.recv(WAIT).unwrap().unwrap().bytes.len(), dev.recv(WAIT).unwrap().unwrap().bytes.len()], [14, 65_536]);
}

#[test]
fn this_machine_s_own_frames_and_two_devices_of_one_name_in_the_snapshot() {
    // Two of this machine's interfaces, lan1 on a segment where lan0's frames arrive too.
    let (quiet, hub) = (Hub::new(), Hub::new());
    let mut other = hub.port("other", Mac([0x02, 0, 0, 0, 0, 0xEE]));
    let l = Listener::start(vec![Box::new(quiet.port("lan0", LAN0)), Box::new(hub.port("lan1", LAN1))], Options::default()).unwrap();
    // This machine's own LLDP agent, its frame from one of its listened MACs naming another of
    // its MACs for its chassis: this machine's, left out.
    let agent = LldpPdu::new(ChassisId::Mac { mac: Mac([0x02, 0, 0, 0, 0, 0xEE]) }, PortId::Local { id: "lan0".into() }, 120);
    let mut from_lan0 = hub.port("lan0-as-sent", LAN0);
    from_lan0.send(&lldp::frame(LAN0, &agent).unwrap()).unwrap();
    // A neighbour's frame naming one of this machine's MACs for its chassis: about this machine.
    let mut naming = LldpPdu::new(ChassisId::Local { id: "switch-1".into() }, PortId::Local { id: "port-003".into() }, 120);
    naming.pn.chassis_mac = Some(LAN1);
    other.send(&lldp::frame(PORT, &naming).unwrap()).unwrap();
    // Two devices of one name.
    other.send(&dcp::hello(DEV, 1, &DeviceIdentity { name: Some("same".into()), ..identity() }).unwrap()).unwrap();
    other.send(&dcp::hello(PORT, 1, &DeviceIdentity { name: Some("same".into()), ..identity() }).unwrap()).unwrap();
    assert!(l.until(WAIT, |s| s.stats.heard == 4));
    let s = l.snapshot();
    assert_eq!((s.stats.own, s.stats.observations), (2, 2));
    assert_eq!(s.fusion.devices.keys().copied().collect::<Vec<_>>(), [DEV, PORT]);
    assert_eq!(s.conflicts, s.fusion.conflicts());
    assert_eq!(s.conflicts.iter().map(|c| (c.kind, c.value.as_str())).collect::<Vec<_>>(), [(veredas_aranha::fusion::ConflictKind::DuplicateName, "same"), (veredas_aranha::fusion::ConflictKind::DuplicateIp, "192.168.0.10")]);
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

#[test]
fn a_tracked_frame_knows_its_place_may_be_withdrawn_and_is_told_even_mid_send() {
    let hub = Hub::new();
    let l = Listener::start(vec![Box::new(hub.port("lan0", LAN0))], Options { rate: 1, ..Options::default() }).unwrap();
    // One sent at once; then two tracked behind it at one a second: nothing ahead of the first,
    // one of the second.
    l.send("lan0", vec![0; 60]).unwrap();
    assert!(l.until(WAIT, |s| s.stats.interfaces["lan0"].sent == 1));
    let first = l.send_tracked("lan0", vec![1; 60]).unwrap();
    let second = l.send_tracked("lan0", vec![2; 60]).unwrap();
    assert_eq!((first.ahead, second.ahead), (0, 1));
    assert_ne!(first.id, second.id);
    // The second withdrawn while it waits: gone from the queue, counted, never told; once.
    assert!(l.withdraw("lan0", second.id));
    assert!(!l.withdraw("lan0", second.id));
    assert!(!l.withdraw("lan9", first.id), "no such interface");
    let i = l.interface("lan0").unwrap();
    assert_eq!((i.queued, i.withdrawn), (1, 1));
    assert_eq!(second.told.recv_timeout(Duration::from_millis(100)), Err(std::sync::mpsc::RecvTimeoutError::Disconnected));
    // The first goes in its turn, then is no longer withdrawn.
    assert!(matches!(first.told.recv_timeout(WAIT), Ok(Outcome::Sent { .. })));
    assert!(!l.withdraw("lan0", first.id));
    assert_eq!(l.interface("lan0").unwrap().sent, 2);
    // A worker that panics as it hands a frame to its wire: a tracked one told never sent with
    // the panic's message, once; an untracked one counted never sent too.
    let l = Listener::start(vec![Box::new(PanicsOnSend)], Options::default()).unwrap();
    let told = l.send_tracked("panics", vec![0; 60]).unwrap().told;
    let reason = "its worker panicked: the card fell out".to_string();
    assert_eq!(told.recv_timeout(WAIT), Ok(Outcome::Unsent(reason.clone())));
    assert_eq!(told.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected), "told once");
    let i = l.interface("panics").unwrap();
    assert_eq!((i.sent, i.unsent, i.ended), (0, 1, Some(reason)));
    let l = Listener::start(vec![Box::new(PanicsOnSend)], Options::default()).unwrap();
    l.send("panics", vec![0; 60]).unwrap();
    assert!(l.until(WAIT, |s| s.stats.interfaces["panics"].ended.is_some()));
    assert_eq!(l.interface("panics").unwrap().unsent, 1);
}
