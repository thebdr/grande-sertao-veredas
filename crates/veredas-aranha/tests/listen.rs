//! The listener (contract: the listener clause): one worker per wire, every frame kept and decoded,
//! its observations fused but for this machine's own, counted by class, damage kept; a paced and
//! bounded sender only where it listens; on the memory hub and a capture replayed.

use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use veredas_aranha::listen::{ListenError, Listener, Options};
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
    listener.send("refusing", vec![0; 60]).unwrap();
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
