//! Core's EtherNet/IP scanner against the bench's EtherNet/IP adapter (contract: the EtherNet/IP
//! scanner clause; the adapter is maquinar's own clause).
//!
//! The adapter is a whole device written apart from the scanner (maquinar's own encapsulation,
//! Connection Manager and Class 1 code; the scanner frames with the `ethernetip-*` crates): its
//! assemblies set and read on the plant side, its knobs stalling production, silencing the
//! session or restarting the device. What the scanner sends lands in the adapter's output
//! assembly, what the plant side sets reaches the scanner's inputs, every refusal comes back with
//! the adapter's own status, and the adapter's own record of the connection is what the scanner
//! asked.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use veredas_core::fieldbus::enip::{EnipError, Header, LinkState, Scanner, Spec, Timeouts};
use veredas_maquinar::ethernetip::{
    self as bench, AssemblyConfig, ConnectionConfig, EnipConfig, EnipHandle,
};

const RPI: Duration = Duration::from_millis(20);

/// A device on a loopback address of its own: an input assembly of 8 bytes (100), an output of 4
/// (150), a configuration of 2 (151), and one connection naming them.
fn device(host: u8) -> EnipHandle {
    let cfg = EnipConfig {
        bind: format!("127.0.0.{host}"),
        port: 0,
        io_port: 0,
        assemblies: vec![
            AssemblyConfig {
                instance: 100,
                size: 8,
            },
            AssemblyConfig {
                instance: 150,
                size: 4,
            },
            AssemblyConfig {
                instance: 151,
                size: 2,
            },
        ],
        connections: vec![ConnectionConfig {
            config: 151,
            output: 150,
            input: 100,
            output_header: None,
            input_header: None,
        }],
        ..EnipConfig::default()
    };
    bench::start(&cfg).unwrap()
}

fn spec(dev: &EnipHandle) -> Spec {
    let mut s = Spec::new(dev.addr, 151, 150, 100, 4, 8);
    s.io_bind = "127.0.0.1:0".parse::<SocketAddr>().unwrap();
    s.rpi = RPI;
    s.vendor = 0x0D0D;
    s.originator_serial = 0x1234_5678;
    s.timeouts = Timeouts {
        connect: Duration::from_millis(500),
        response: Duration::from_millis(300),
    };
    s
}

fn wait(within: Duration, mut what: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < within {
        if what() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    what()
}

#[test]
fn outputs_land_in_the_adapter_and_its_inputs_reach_the_scanner_at_the_interval() {
    let dev = device(171);
    let scanner = Scanner::open(&spec(&dev)).unwrap();
    let opened = scanner.opened();
    assert_eq!((opened.o_to_t_rpi, opened.t_to_o_rpi), (RPI, RPI));
    assert_eq!(opened.timeout, RPI * 4);
    assert_eq!(
        opened.sends_to, dev.io_addr,
        "the adapter's I/O address, from its reply's item 0x8000"
    );
    // The adapter's own record of the connection: what the scanner asked.
    let status = dev.status();
    assert_eq!(status.connections.len(), 1);
    let c = &status.connections[0];
    assert_eq!((c.config, c.output, c.input), (151, 150, 100));
    assert_eq!(
        (c.o_to_t_id, c.t_to_o_id),
        (opened.o_to_t_id, opened.t_to_o_id)
    );
    assert_eq!(
        (c.serial, c.vendor, c.originator_serial),
        (opened.connection_serial, 0x0D0D, 0x1234_5678)
    );
    assert_eq!((c.o_to_t_rpi_us, c.t_to_o_rpi_us), (20_000, 20_000));
    assert_eq!(
        c.sends_to, opened.receives_on,
        "the scanner's I/O address, from its request's item 0x8001"
    );
    // Outputs: idle at first, so the adapter keeps its output; run, and they land.
    scanner.set_outputs(&[1, 2, 3, 4]).unwrap();
    assert!(wait(Duration::from_secs(1), || dev.status().connections[0]
        .run
        == Some(false)));
    assert_eq!(
        dev.assembly(150).unwrap(),
        vec![0; 4],
        "idle data is not applied"
    );
    scanner.set_run(true);
    assert!(wait(Duration::from_secs(1), || dev.assembly(150).unwrap()
        == [1, 2, 3, 4]));
    assert_eq!(dev.status().connections[0].run, Some(true));
    // Idle again: the next outputs do not land.
    scanner.set_run(false);
    assert!(wait(Duration::from_secs(1), || dev.status().connections[0]
        .run
        == Some(false)));
    scanner.set_outputs(&[9, 9, 9, 9]).unwrap();
    std::thread::sleep(RPI * 5);
    assert_eq!(dev.assembly(150).unwrap(), vec![1, 2, 3, 4]);
    // Inputs: what the plant side sets.
    dev.set_assembly(100, &[10, 20, 30, 40, 50, 60, 70, 80])
        .unwrap();
    assert!(wait(Duration::from_secs(1), || scanner.inputs().data
        == [10, 20, 30, 40, 50, 60, 70, 80]));
    assert_eq!(
        scanner.inputs().run,
        None,
        "modeless inputs carry no run/idle header"
    );
    // A second, both directions at their interval: 1 s of 20 ms is 50 frames each way.
    let (p0, c0) = (scanner.counters().produced, scanner.counters().consumed);
    let a0 = dev.status().connections[0].consumed;
    std::thread::sleep(Duration::from_secs(1));
    let (p1, c1) = (scanner.counters().produced, scanner.counters().consumed);
    let a1 = dev.status().connections[0].consumed;
    assert!((40..=60).contains(&(p1 - p0)), "produced {}", p1 - p0);
    assert!((40..=60).contains(&(c1 - c0)), "consumed {}", c1 - c0);
    assert!(
        (40..=60).contains(&(a1 - a0)),
        "the adapter consumed {}",
        a1 - a0
    );
    assert_eq!(scanner.counters().stray, 0);
    assert_eq!(dev.status().stray_frames, 0);
    assert_eq!(scanner.state(), LinkState::Running);
    // A wrong size of outputs is refused at the handle.
    assert!(matches!(
        scanner.set_outputs(&[1, 2, 3]),
        Err(EnipError::Refused { .. })
    ));
    // Closing: a Forward_Close the adapter confirms, its output free at once.
    scanner.close().unwrap();
    let status = dev.status();
    assert!(status.connections.is_empty());
    assert_eq!((status.closed, status.timed_out), (1, 0));
    let again = Scanner::open(&spec(&dev)).unwrap();
    assert_eq!(again.state(), LinkState::Running);
}

#[test]
fn the_adapters_refusals_come_back_with_its_status() {
    let dev = device(172);
    let refused = |s: &Spec| match Scanner::open(s) {
        Err(EnipError::Open { general, extended }) => (general, extended.first().copied()),
        other => panic!("{other:?}"),
    };
    let mut s = spec(&dev);
    s.output_size = 5;
    assert_eq!(
        refused(&s),
        (0x01, Some(0x0127)),
        "an O→T size the output does not have"
    );
    let mut s = spec(&dev);
    s.input_size = 6;
    assert_eq!(
        refused(&s),
        (0x01, Some(0x0128)),
        "a T→O size the input does not have"
    );
    let mut s = spec(&dev);
    s.output = 160;
    assert_eq!(
        refused(&s),
        (0x01, Some(0x012A)),
        "a consuming path to no assembly"
    );
    let mut s = spec(&dev);
    s.input = 160;
    assert_eq!(
        refused(&s),
        (0x01, Some(0x012B)),
        "a producing path to no assembly"
    );
    let mut s = spec(&dev);
    s.rpi = Duration::from_micros(500);
    assert_eq!(
        refused(&s),
        (0x01, Some(0x0111)),
        "an interval below the adapter's least"
    );
    let mut s = spec(&dev);
    s.input_header = Header::RunIdle;
    assert_eq!(
        refused(&s),
        (0x01, Some(0x0128)),
        "inputs with a header the adapter does not produce"
    );
    // An output already owned by another open connection.
    let first = Scanner::open(&spec(&dev)).unwrap();
    assert_eq!(
        refused(&spec(&dev)),
        (0x01, Some(0x0106)),
        "an output owned"
    );
    assert_eq!(dev.status().refused_opens.get(&0x0106), Some(&1));
    let e = EnipError::Open {
        general: 0x01,
        extended: vec![0x0106],
    };
    assert_eq!(
        e.to_string(),
        "the adapter refused the connection: general status 0x01, extended 0x0106"
    );
    drop(first);
    assert!(
        wait(Duration::from_secs(1), || dev
            .status()
            .connections
            .is_empty()),
        "a drop closes it"
    );
    assert_eq!(dev.status().closed, 1);
}

#[test]
fn a_stalled_adapter_loses_the_connection_at_its_timeout() {
    let dev = device(173);
    let scanner = Scanner::open(&spec(&dev)).unwrap();
    assert!(wait(Duration::from_secs(1), || scanner.inputs().updates > 2));
    dev.set_producing(false);
    let stalled = Instant::now();
    assert!(wait(Duration::from_secs(1), || scanner.state()
        != LinkState::Running));
    let took = stalled.elapsed();
    assert_eq!(scanner.state(), LinkState::Lost { timeout: RPI * 4 });
    assert!(
        took >= RPI * 3 && took < RPI * 4 + Duration::from_millis(200),
        "{took:?}"
    );
    // Lost: the scanner produces no more, so the adapter times its end out too.
    let produced = scanner.counters().produced;
    std::thread::sleep(RPI * 5);
    assert_eq!(scanner.counters().produced, produced);
    assert!(wait(Duration::from_secs(1), || dev.status().timed_out == 1));
    assert!(dev.status().connections.is_empty());
    // Closing a lost connection ends it here; nothing is asked of the adapter.
    scanner.close().unwrap();
}

#[test]
fn a_restarted_adapter_loses_the_connection_and_a_new_one_opens() {
    let dev = device(174);
    let scanner = Scanner::open(&spec(&dev)).unwrap();
    assert!(wait(Duration::from_secs(1), || scanner.inputs().updates > 2));
    dev.restart();
    assert!(wait(Duration::from_secs(1), || matches!(
        scanner.state(),
        LinkState::Lost { .. }
    )));
    drop(scanner);
    let again = Scanner::open(&spec(&dev)).unwrap();
    assert!(wait(Duration::from_secs(1), || again.inputs().updates > 2));
}

#[test]
fn a_silent_session_bounds_the_open_and_a_gone_adapter_refuses_it() {
    let dev = device(175);
    dev.set_silent(true);
    let started = Instant::now();
    let e = Scanner::open(&spec(&dev)).unwrap_err();
    assert_eq!(
        e,
        EnipError::NoAnswer {
            step: "RegisterSession",
            cause: "no answer within 300 ms".into()
        }
    );
    assert!(
        started.elapsed() < Duration::from_millis(900),
        "{:?}",
        started.elapsed()
    );
    dev.set_silent(false);
    let addr = dev.addr;
    dev.stop();
    let mut s = Spec::new(addr, 151, 150, 100, 4, 8);
    s.io_bind = "127.0.0.1:0".parse::<SocketAddr>().unwrap();
    assert!(matches!(
        Scanner::open(&s),
        Err(EnipError::NotConnected { .. })
    ));
}
