//! The EtherNet/IP scanner (contract: the EtherNet/IP scanner clause), over real TCP and UDP on
//! loopback.
//!
//! The adapter here is scripted: it records every byte the scanner sends and answers as each test
//! tells it, so each request is held to the standard's layout written out by hand below (CIP
//! Volume 1's Forward_Open and Forward_Close, Volume 2's encapsulation, Common Packet Format and
//! Class 1 frames), every refusal and failure is played, and the whole conversation is handed to
//! Wireshark's dissectors, a reading of the standard written apart from the scanner, its codecs
//! and the bench. The scanner against a whole adapter, the bench's, is the clause's other test.

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::Value as Json;
use veredas_core::fieldbus::enip::*;

const RPI: Duration = Duration::from_millis(20);
const HANDLE: u32 = 0x1122_3344;
const O_TO_T_ID: u32 = 0xAAAA_0001;

/// Frames or datagrams in order, true from the scanner.
type Conversation = Vec<(bool, Vec<u8>)>;

/// What the scripted adapter answers.
#[derive(Clone)]
enum Answer {
    /// The standard's answer.
    Normal,
    /// Nothing at all.
    Silent,
    /// An encapsulation status in place of the answer.
    Status(u32),
    /// A general status and its extended words.
    Refuse(u8, Vec<u16>),
    /// These bytes in place of the Message Router reply.
    Reply(Vec<u8>),
    /// The answer with its sender context changed.
    OtherContext,
}

#[derive(Clone)]
struct Script {
    register: Answer,
    open: Answer,
    close: Answer,
    /// The granted intervals, in microseconds; the requested ones when `None`.
    granted: Option<(u32, u32)>,
    /// Whether the reply names the adapter's I/O address (item 0x8000).
    names_io: bool,
    /// The address the 0x8000 item names, the adapter's I/O socket's when `None`.
    io_named: Option<SocketAddr>,
    /// How long the Forward_Open's answer is held back.
    open_delay: Duration,
    /// Inputs carry a run/idle header.
    input_run_idle: bool,
    input_size: usize,
}

impl Default for Script {
    fn default() -> Script {
        Script {
            register: Answer::Normal,
            open: Answer::Normal,
            close: Answer::Normal,
            granted: None,
            names_io: true,
            io_named: None,
            open_delay: Duration::ZERO,
            input_run_idle: false,
            input_size: 6,
        }
    }
}

/// A scripted adapter on 127.0.0.1: its session on TCP, its I/O on UDP.
struct Adapter {
    addr: SocketAddr,
    io: Arc<UdpSocket>,
    /// TCP frames in order, true from the scanner.
    log: Arc<Mutex<Conversation>>,
    /// Datagrams in order, true from the scanner.
    datagrams: Arc<Mutex<Conversation>>,
    producing: Arc<AtomicBool>,
    /// The next input data, and its sequence count bumped on each set.
    inputs: Arc<Mutex<(Vec<u8>, u16)>>,
    connections: Arc<AtomicU32>,
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn encap(command: u16, handle: u32, status: u32, context: &[u8], data: &[u8]) -> Vec<u8> {
    let mut f = command.to_le_bytes().to_vec();
    f.extend_from_slice(&(data.len() as u16).to_le_bytes());
    f.extend_from_slice(&handle.to_le_bytes());
    f.extend_from_slice(&status.to_le_bytes());
    f.extend_from_slice(context);
    f.extend_from_slice(&0u32.to_le_bytes());
    f.extend_from_slice(data);
    f
}

/// A SendRRData body: interface 0, timeout 0, the items.
fn rr(items: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut b = vec![0, 0, 0, 0, 0, 0];
    b.extend_from_slice(&(items.len() as u16).to_le_bytes());
    for (t, d) in items {
        b.extend_from_slice(&t.to_le_bytes());
        b.extend_from_slice(&(d.len() as u16).to_le_bytes());
        b.extend_from_slice(d);
    }
    b
}

fn sockaddr(a: SocketAddr) -> Vec<u8> {
    let mut s = vec![0u8; 16];
    s[0..2].copy_from_slice(&2u16.to_be_bytes());
    s[2..4].copy_from_slice(&a.port().to_be_bytes());
    if let IpAddr::V4(ip) = a.ip() {
        s[4..8].copy_from_slice(&ip.octets());
    }
    s
}

/// The items of a SendRRData body, by type.
fn items(body: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let n = le16(body, 6) as usize;
    let mut at = 8;
    let mut out = Vec::new();
    for _ in 0..n {
        let (t, len) = (le16(body, at), le16(body, at + 2) as usize);
        out.push((t, body[at + 4..at + 4 + len].to_vec()));
        at += 4 + len;
    }
    out
}

impl Adapter {
    fn start(script: Script) -> Adapter {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let io = Arc::new(UdpSocket::bind("127.0.0.1:0").unwrap());
        let a = Adapter {
            addr,
            io: io.clone(),
            log: Arc::default(),
            datagrams: Arc::default(),
            producing: Arc::new(AtomicBool::new(true)),
            inputs: Arc::new(Mutex::new((vec![0; script.input_size], 0))),
            connections: Arc::default(),
        };
        // The scanner's outputs, recorded.
        let (rx, recorded) = (io.clone(), a.datagrams.clone());
        thread::spawn(move || {
            let mut buf = [0u8; 1500];
            while let Ok((n, _)) = rx.recv_from(&mut buf) {
                recorded.lock().unwrap().push((true, buf[..n].to_vec()));
            }
        });
        let (log, datagrams, producing, inputs, connections) = (
            a.log.clone(),
            a.datagrams.clone(),
            a.producing.clone(),
            a.inputs.clone(),
            a.connections.clone(),
        );
        thread::spawn(move || {
            for conn in listener.incoming() {
                let Ok(mut s) = conn else { continue };
                connections.fetch_add(1, Ordering::SeqCst);
                let (log, datagrams, producing, inputs, io, script) = (
                    log.clone(),
                    datagrams.clone(),
                    producing.clone(),
                    inputs.clone(),
                    io.clone(),
                    script.clone(),
                );
                thread::spawn(move || {
                    serve(&mut s, &script, &log, &datagrams, &producing, &inputs, &io)
                });
            }
        });
        a
    }

    fn frames(&self) -> Vec<Vec<u8>> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|(mine, _)| *mine)
            .map(|(_, f)| f.clone())
            .collect()
    }

    fn outputs(&self) -> Vec<Vec<u8>> {
        self.datagrams
            .lock()
            .unwrap()
            .iter()
            .filter(|(mine, _)| *mine)
            .map(|(_, f)| f.clone())
            .collect()
    }

    fn set_inputs(&self, data: &[u8]) {
        let mut i = self.inputs.lock().unwrap();
        i.0 = data.to_vec();
        i.1 = i.1.wrapping_add(1);
    }
}

#[allow(clippy::too_many_arguments)]
fn serve(
    s: &mut TcpStream,
    script: &Script,
    log: &Mutex<Conversation>,
    datagrams: &Arc<Mutex<Conversation>>,
    producing: &Arc<AtomicBool>,
    inputs: &Arc<Mutex<(Vec<u8>, u16)>>,
    io: &Arc<UdpSocket>,
) {
    let stop = Arc::new(AtomicBool::new(false));
    loop {
        let mut h = [0u8; 24];
        if s.read_exact(&mut h).is_err() {
            break;
        }
        let mut data = vec![0u8; le16(&h, 2) as usize];
        if s.read_exact(&mut data).is_err() {
            break;
        }
        let frame = [h.to_vec(), data.clone()].concat();
        log.lock().unwrap().push((true, frame));
        let (command, context) = (le16(&h, 0), h[12..20].to_vec());
        let other = |c: &[u8]| -> Vec<u8> { c.iter().map(|b| b ^ 0xFF).collect() };
        let answer = |answer: &Answer, body: Vec<u8>| -> Option<Vec<u8>> {
            match answer {
                Answer::Silent => None,
                Answer::Status(st) => Some(encap(command, HANDLE, *st, &context, &[])),
                Answer::OtherContext => Some(encap(command, HANDLE, 0, &other(&context), &body)),
                _ => Some(encap(command, HANDLE, 0, &context, &body)),
            }
        };
        let reply = match command {
            0x0065 => answer(&script.register, data.clone()),
            0x0066 => break,
            0x006F => {
                let items = items(&data);
                let mr = &items.iter().find(|(t, _)| *t == 0x00B2).unwrap().1;
                let (service, body) = (mr[0], &mr[6..]);
                let which = if service == 0x54 {
                    &script.open
                } else {
                    &script.close
                };
                let mr_reply = match which {
                    Answer::Refuse(g, ext) => {
                        let mut r = vec![service | 0x80, 0, *g, ext.len() as u8];
                        for e in ext {
                            r.extend_from_slice(&e.to_le_bytes());
                        }
                        r
                    }
                    Answer::Reply(bytes) => bytes.clone(),
                    _ if service == 0x54 => {
                        let (ot_rpi, to_rpi) =
                            script.granted.unwrap_or((le32(body, 22), le32(body, 28)));
                        let mut r = vec![0xD4, 0, 0, 0];
                        r.extend_from_slice(&O_TO_T_ID.to_le_bytes());
                        r.extend_from_slice(&body[6..10]); // the T→O id the scanner chose
                        r.extend_from_slice(&body[10..18]); // its serial, vendor and serial number
                        r.extend_from_slice(&ot_rpi.to_le_bytes());
                        r.extend_from_slice(&to_rpi.to_le_bytes());
                        r.extend_from_slice(&[0, 0]);
                        // Produce the inputs to where item 0x8001 says, at the granted interval.
                        let t_to_o_id = le32(body, 6);
                        let to = items.iter().find(|(t, _)| *t == 0x8001).map(|(_, d)| {
                            SocketAddr::new(
                                IpAddr::V4(Ipv4Addr::new(d[4], d[5], d[6], d[7])),
                                u16::from_be_bytes([d[2], d[3]]),
                            )
                        });
                        if let Some(to) = to {
                            let (io, producing, inputs, datagrams, stop) = (
                                io.clone(),
                                producing.clone(),
                                inputs.clone(),
                                datagrams.clone(),
                                stop.clone(),
                            );
                            let run_idle = script.input_run_idle;
                            thread::spawn(move || {
                                let mut seq = 0u32;
                                while !stop.load(Ordering::SeqCst) {
                                    thread::sleep(Duration::from_micros(u64::from(to_rpi)));
                                    if !producing.load(Ordering::SeqCst) {
                                        continue;
                                    }
                                    seq += 1;
                                    let (data, count) = inputs.lock().unwrap().clone();
                                    let f = input_frame(
                                        t_to_o_id,
                                        seq,
                                        count,
                                        run_idle.then_some(true),
                                        &data,
                                    );
                                    let _ = io.send_to(&f, to);
                                    datagrams.lock().unwrap().push((false, f));
                                }
                            });
                        }
                        r
                    }
                    _ => {
                        stop.store(true, Ordering::SeqCst);
                        let mut r = vec![0xCE, 0, 0, 0];
                        r.extend_from_slice(&body[2..10]);
                        r.extend_from_slice(&[0, 0]);
                        r
                    }
                };
                let mut reply_items = vec![(0x0000, vec![]), (0x00B2, mr_reply)];
                if service == 0x54 && script.names_io {
                    let named = script.io_named.unwrap_or_else(|| io.local_addr().unwrap());
                    reply_items.push((0x8000, sockaddr(named)));
                }
                answer(which, rr(&reply_items))
            }
            _ => None,
        };
        if command == 0x006F
            && items(&data)
                .iter()
                .any(|(t, d)| *t == 0x00B2 && d.first() == Some(&0x54))
        {
            thread::sleep(script.open_delay);
        }
        if let Some(r) = reply {
            log.lock().unwrap().push((false, r.clone()));
            if s.write_all(&r).is_err() {
                break;
            }
        }
    }
    stop.store(true, Ordering::SeqCst);
}

/// A Class 1 frame as the standard lays it out: two items, the sequenced address (connection id
/// and the encapsulation sequence) and the connected data (the 16-bit count, a header, the data).
fn input_frame(id: u32, seq: u32, count: u16, run: Option<bool>, data: &[u8]) -> Vec<u8> {
    let mut f = vec![2, 0, 0x02, 0x80, 8, 0];
    f.extend_from_slice(&id.to_le_bytes());
    f.extend_from_slice(&seq.to_le_bytes());
    f.extend_from_slice(&[0xB1, 0x00]);
    let len = 2 + if run.is_some() { 4 } else { 0 } + data.len();
    f.extend_from_slice(&(len as u16).to_le_bytes());
    f.extend_from_slice(&count.to_le_bytes());
    if let Some(r) = run {
        f.extend_from_slice(&u32::from(r).to_le_bytes());
    }
    f.extend_from_slice(data);
    f
}

/// The spec with a short response bound, for the tests that time the bound itself.
fn spec_short(a: &Adapter) -> Spec {
    let mut s = spec(a);
    s.timeouts.response = Duration::from_millis(300);
    s
}

/// Waits for the scanner's UnRegisterSession to reach the adapter (its last frame).
fn unregistered(a: &Adapter) -> bool {
    wait(Duration::from_secs(1), || {
        a.frames().last().map(|f| le16(f, 0)) == Some(0x0066)
    })
}

fn spec(a: &Adapter) -> Spec {
    let mut s = Spec::new(a.addr, 151, 150, 100, 4, 6);
    s.io_bind = "127.0.0.1:0".parse().unwrap();
    s.rpi = RPI;
    s.vendor = 0x0D0D;
    s.originator_serial = 0x1234_5678;
    s.timeouts = Timeouts {
        connect: Duration::from_millis(500),
        // Long enough for a loaded machine; a test of the bound itself sets its own.
        response: Duration::from_secs(1),
    };
    s
}

fn wait(within: Duration, mut what: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    while started.elapsed() < within {
        if what() {
            return true;
        }
        thread::sleep(Duration::from_millis(5));
    }
    what()
}

/// The Forward_Open the scanner must send for `spec(..)` with this serial, byte for byte.
fn forward_open_bytes(serial: u16, receives_on: SocketAddr, context: u8) -> Vec<u8> {
    let mut fo = vec![0x54, 0x02, 0x20, 0x06, 0x24, 0x01]; // Forward_Open to the Connection Manager
    fo.extend_from_slice(&[0x0A, 0x0E]); // priority and tick, timeout ticks
    fo.extend_from_slice(&0u32.to_le_bytes()); // O→T id: the adapter's to choose
    fo.extend_from_slice(&(0x8000_0000 | u32::from(serial)).to_le_bytes()); // T→O id: ours
    fo.extend_from_slice(&serial.to_le_bytes());
    fo.extend_from_slice(&0x0D0Du16.to_le_bytes());
    fo.extend_from_slice(&0x1234_5678u32.to_le_bytes());
    fo.extend_from_slice(&[0, 0, 0, 0]); // multiplier 0 (×4), reserved
    fo.extend_from_slice(&20_000u32.to_le_bytes());
    // Point to point (2 << 13), scheduled (2 << 10), fixed, 4 data + 2 count + 4 run/idle = 10.
    fo.extend_from_slice(&(0x4000u16 | 0x0800 | 10).to_le_bytes());
    fo.extend_from_slice(&20_000u32.to_le_bytes());
    fo.extend_from_slice(&(0x4000u16 | 0x0800 | 8).to_le_bytes()); // 6 data + 2 count, modeless
    fo.push(0x01); // Class 1, cyclic, client
    fo.extend_from_slice(&[4, 0x20, 0x04, 0x24, 151, 0x2C, 150, 0x2C, 100]);
    let body = rr(&[
        (0x0000, vec![]),
        (0x00B2, fo),
        (0x8001, sockaddr(receives_on)),
    ]);
    let mut ctx = [0u8; 8];
    ctx[0] = context;
    encap(0x006F, HANDLE, 0, &ctx, &body)
}

#[test]
fn every_request_is_the_standards_bytes_and_the_exchange_runs_at_the_interval() {
    let a = Adapter::start(Script::default());
    let s = Scanner::open(&spec(&a)).unwrap();
    let opened = s.opened();
    assert_eq!(opened.o_to_t_id, O_TO_T_ID);
    assert_eq!(
        opened.t_to_o_id,
        0x8000_0000 | u32::from(opened.connection_serial)
    );
    assert_eq!(
        (opened.o_to_t_rpi, opened.t_to_o_rpi, opened.timeout),
        (RPI, RPI, RPI * 4)
    );
    assert_eq!(
        opened.sends_to,
        a.io.local_addr().unwrap(),
        "where item 0x8000 says"
    );
    // RegisterSession: version 1, options 0, the first sender context.
    let frames = a.frames();
    assert_eq!(
        frames[0],
        encap(0x0065, 0, 0, &[1, 0, 0, 0, 0, 0, 0, 0], &[1, 0, 0, 0])
    );
    assert_eq!(
        frames[1],
        forward_open_bytes(opened.connection_serial, opened.receives_on, 2)
    );
    // Outputs: idle, then run; the frame as the standard lays it out.
    s.set_outputs(&[1, 2, 3, 4]).unwrap();
    assert!(wait(Duration::from_secs(1), || a.outputs().len() > 3));
    s.set_run(true);
    thread::sleep(RPI * 3);
    let outs = a.outputs();
    let first = &outs[0];
    assert_eq!(
        &first[..6],
        [2, 0, 0x02, 0x80, 8, 0],
        "two items, the sequenced address first"
    );
    assert_eq!(le32(first, 6), O_TO_T_ID);
    assert_eq!(
        &first[14..18],
        [0xB1, 0x00, 10, 0],
        "connected data: count, header, 4 bytes"
    );
    let last = outs.last().unwrap();
    assert_eq!(&last[20..24], [1, 0, 0, 0], "run");
    assert_eq!(&last[24..], [1, 2, 3, 4]);
    assert_eq!(&first[20..24], [0, 0, 0, 0], "idle at first");
    // Each frame counts on: the encapsulation sequence and the 16-bit count.
    for w in outs.windows(2) {
        assert_eq!(le32(&w[1], 10), le32(&w[0], 10) + 1);
        assert_eq!(le16(&w[1], 18), le16(&w[0], 18).wrapping_add(1));
    }
    // Inputs: the adapter's data, a new count each change.
    a.set_inputs(&[9, 8, 7, 6, 5, 4]);
    assert!(wait(Duration::from_secs(1), || s.inputs().data
        == [9, 8, 7, 6, 5, 4]));
    // About 50 frames a second each way at 20 ms.
    let (p0, c0) = (s.counters().produced, s.counters().consumed);
    thread::sleep(Duration::from_secs(1));
    let (p1, c1) = (s.counters().produced, s.counters().consumed);
    assert!((40..=60).contains(&(p1 - p0)), "produced {}", p1 - p0);
    assert!((35..=60).contains(&(c1 - c0)), "consumed {}", c1 - c0);
    // A repeated count is no new data: the adapter repeats its count until the next set.
    let updates = s.inputs().updates;
    thread::sleep(RPI * 5);
    assert_eq!(s.inputs().updates, updates);
    // Close: Forward_Close, then UnRegisterSession, as the standard lays them out.
    let serial = opened.connection_serial;
    s.close().unwrap();
    assert!(unregistered(&a));
    let frames = a.frames();
    let mut fc = vec![0x4E, 0x02, 0x20, 0x06, 0x24, 0x01, 0x0A, 0x0E];
    fc.extend_from_slice(&serial.to_le_bytes());
    fc.extend_from_slice(&0x0D0Du16.to_le_bytes());
    fc.extend_from_slice(&0x1234_5678u32.to_le_bytes());
    fc.extend_from_slice(&[4, 0, 0x20, 0x04, 0x24, 151, 0x2C, 150, 0x2C, 100]);
    let mut ctx = [0u8; 8];
    ctx[0] = 3;
    assert_eq!(
        frames[2],
        encap(
            0x006F,
            HANDLE,
            0,
            &ctx,
            &rr(&[(0x0000, vec![]), (0x00B2, fc)])
        )
    );
    assert_eq!(frames[3], encap(0x0066, HANDLE, 0, &[0; 8], &[]));
    assert_eq!(frames.len(), 4);
    thread::sleep(RPI); // a frame already on its way is recorded before the count
    let produced = a.outputs().len();
    thread::sleep(RPI * 3);
    assert_eq!(
        a.outputs().len(),
        produced,
        "nothing produced after the close"
    );
}

#[test]
fn the_dissectors_read_the_whole_conversation_as_the_standard_has_it() {
    let a = Adapter::start(Script::default());
    let s = Scanner::open(&spec(&a)).unwrap();
    s.set_outputs(&[1, 0, 0x34, 0x12]).unwrap();
    s.set_run(true);
    a.set_inputs(&[1, 2, 3, 4, 5, 6]);
    assert!(wait(Duration::from_secs(1), || s.counters().consumed > 2
        && a.outputs().len() > 2));
    let opened = s.opened();
    s.close().unwrap();
    assert!(unregistered(&a));
    let log = a.log.lock().unwrap().clone();
    let datagrams: Vec<(bool, Vec<u8>)> = a
        .datagrams
        .lock()
        .unwrap()
        .iter()
        .take(8)
        .cloned()
        .collect();
    let packets = dissect(&capture(&log, &datagrams));
    let enip: Vec<&Json> = packets.iter().filter(|p| !p["enip"].is_null()).collect();
    assert_eq!(
        enip.len(),
        log.len() + datagrams.len(),
        "every frame read as EtherNet/IP"
    );
    for p in &packets {
        assert_eq!(complaints(p), Vec::<String>::new());
    }
    let all = |key: &str| {
        packets
            .iter()
            .flat_map(|p| field(p, key))
            .collect::<Vec<_>>()
    };
    // The Forward_Open as the dissector reads it.
    assert_eq!(all("cipcm_cip_cm_fwo_consize"), ["10", "8"]);
    assert_eq!(all("cipcm_cip_cm_otrpi"), ["20000"]);
    assert_eq!(all("cipcm_cip_cm_torpi"), ["20000"]);
    assert_eq!(
        all("cipcm_cip_cm_conn_serial_num").first(),
        Some(&opened.connection_serial.to_string())
    );
    // The Sockaddr Info items by the standard's numbers: 0x8001 (where T→O data goes) in the
    // request, 0x8000 (where O→T data goes) in the reply, each with its port.
    let types = all("enip_enip_cpf_typeid");
    assert!(
        types.contains(&"32769".to_string()) && types.contains(&"32768".to_string()),
        "{types:?}"
    );
    let ports = all("enip_enip_sinport");
    assert_eq!(
        ports,
        [
            opened.receives_on.port().to_string(),
            a.io.local_addr().unwrap().port().to_string()
        ]
    );
    let statuses = all("cip_cip_genstat");
    assert!(
        !statuses.is_empty() && statuses.iter().all(|g| g == "0"),
        "{statuses:?}"
    );
}

#[test]
fn a_connection_outside_the_standards_bounds_never_reaches_the_wire() {
    let a = Adapter::start(Script::default());
    let refused = |f: fn(&mut Spec)| {
        let mut s = spec(&a);
        f(&mut s);
        match Scanner::open(&s) {
            Err(EnipError::Refused { reason }) => reason,
            other => panic!("{other:?}"),
        }
    };
    let reasons = vec![
        refused(|s| s.output_size = 506),
        refused(|s| s.input_size = 510),
        refused(|s| {
            s.input_size = 506;
            s.input_header = Header::RunIdle
        }),
        refused(|s| s.rpi = Duration::ZERO),
        refused(|s| s.rpi = Duration::from_secs(4295)),
        refused(|s| s.timeout_multiplier = 8),
        refused(|s| s.io_bind = "[::]:0".parse().unwrap()),
        refused(|s| s.adapter = "[::1]:44818".parse().unwrap()),
    ];
    assert_eq!(
        reasons,
        vec![
            "506 output bytes with their count and header pass the 511 bytes a Forward_Open carries",
            "510 input bytes with their count and header pass the 511 bytes a Forward_Open carries",
            "506 input bytes with their count and header pass the 511 bytes a Forward_Open carries",
            "an interval of 0 µs: the standard takes 1 to 4294967295",
            "an interval of 4295000000 µs: the standard takes 1 to 4294967295",
            "a timeout multiplier of 8: the standard takes 0 to 7",
            "the I/O socket [::]:0: EtherNet/IP's I/O addresses are IPv4",
            "the adapter [::1]:44818: EtherNet/IP's I/O addresses are IPv4",
        ]
    );
    assert_eq!(
        a.connections.load(Ordering::SeqCst),
        0,
        "nothing sent, not even a connection"
    );
    // An IPv6 adapter that is listening: refused before any connection reaches it.
    let v6 = TcpListener::bind("[::1]:0").unwrap();
    let mut s6 = spec(&a);
    s6.adapter = v6.local_addr().unwrap();
    assert!(matches!(Scanner::open(&s6), Err(EnipError::Refused { .. })));
    v6.set_nonblocking(true).unwrap();
    assert_eq!(
        v6.accept().map(|_| ()).map_err(|e| e.kind()),
        Err(std::io::ErrorKind::WouldBlock),
        "no connection reached the IPv6 adapter"
    );
    // The limits taken: 505 bytes with their count and header, multiplier 7.
    let mut s = spec(&a);
    (s.output_size, s.timeout_multiplier) = (505, 7);
    let big = Scanner::open(&s);
    let Ok(big) = big else { panic!("{big:?}") };
    assert_eq!(big.opened().timeout, RPI * 512);
    // Instances past 255 take the 16-bit segments.
    let mut s = spec(&a);
    (s.config, s.output, s.input) = (0x1234, 300, 7);
    assert_eq!(
        s.connection_path(),
        [
            0x20, 0x04, 0x25, 0, 0x34, 0x12, 0x2D, 0, 0x2C, 0x01, 0x2C, 7
        ]
    );
}

#[test]
fn the_adapters_refusals_and_wrong_answers_end_the_open_by_name() {
    let open = |script: Script| Scanner::open(&spec(&Adapter::start(script))).unwrap_err();
    let open_short =
        |script: Script| Scanner::open(&spec_short(&Adapter::start(script))).unwrap_err();
    assert_eq!(
        open(Script {
            open: Answer::Refuse(0x01, vec![0x0127, 10]),
            ..Script::default()
        }),
        EnipError::Open {
            general: 0x01,
            extended: vec![0x0127, 10]
        }
    );
    assert_eq!(
        EnipError::Open {
            general: 0x01,
            extended: vec![0x0127, 10]
        }
        .to_string(),
        "the adapter refused the connection: general status 0x01, extended [0127, 000a]"
    );
    assert_eq!(
        open(Script {
            open: Answer::Refuse(0x08, vec![]),
            ..Script::default()
        })
        .to_string(),
        "the adapter refused the connection: general status 0x08"
    );
    assert_eq!(
        open(Script {
            register: Answer::Status(0x69),
            ..Script::default()
        }),
        EnipError::Session { status: 0x69 }
    );
    let no_answer = |e: EnipError| match e {
        EnipError::NoAnswer { step, cause } => format!("{step}: {cause}"),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        no_answer(open_short(Script {
            register: Answer::Silent,
            ..Script::default()
        })),
        "RegisterSession: no answer within 300 ms"
    );
    assert_eq!(
        no_answer(open_short(Script {
            open: Answer::Silent,
            ..Script::default()
        })),
        "Forward_Open: no answer within 300 ms"
    );
    assert_eq!(
        no_answer(open(Script {
            register: Answer::OtherContext,
            ..Script::default()
        })),
        "RegisterSession: an answer for another request (its sender context)"
    );
    assert_eq!(
        no_answer(open(Script {
            open: Answer::Status(0x03),
            ..Script::default()
        })),
        "Forward_Open: encapsulation status 0x00000003"
    );
    assert_eq!(
        no_answer(open(Script {
            open: Answer::Reply(vec![0xCE, 0, 0, 0]),
            ..Script::default()
        })),
        "Forward_Open: a reply to another service (Some(ce))"
    );
    let mut other_originator = vec![0xD4, 0, 0, 0];
    other_originator.extend_from_slice(&[0; 26]);
    assert_eq!(
        no_answer(open(Script {
            open: Answer::Reply(other_originator),
            ..Script::default()
        })),
        "Forward_Open: a reply for another originator's connection"
    );
    assert_eq!(
        no_answer(open(Script {
            granted: Some((20_000, 0)),
            ..Script::default()
        })),
        "Forward_Open: a granted interval of 0"
    );
    // Connecting: a port nobody listens on, at once; a full queue, at the bound.
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let mut s = Spec::new(closed, 1, 2, 3, 0, 0);
    s.io_bind = "127.0.0.1:0".parse().unwrap();
    s.timeouts.connect = Duration::from_millis(500);
    assert!(
        matches!(Scanner::open(&s), Err(EnipError::NotConnected { addr, .. }) if addr == closed)
    );
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    socket
        .bind(&"127.0.0.1:0".parse::<SocketAddr>().unwrap().into())
        .unwrap();
    socket.listen(0).unwrap();
    let full = socket.local_addr().unwrap().as_socket().unwrap();
    let held: Vec<_> = (0..4)
        .filter_map(|_| TcpStream::connect_timeout(&full, Duration::from_millis(100)).ok())
        .collect();
    s.adapter = full;
    let t = Instant::now();
    assert_eq!(
        Scanner::open(&s).unwrap_err(),
        EnipError::NotConnected {
            addr: full,
            reason: "no connection within 500 ms".into()
        }
    );
    assert!(
        t.elapsed() < Duration::from_millis(1100),
        "{:?}",
        t.elapsed()
    );
    drop(held);
    // An I/O port taken: nothing opened.
    let a = Adapter::start(Script::default());
    let taken = UdpSocket::bind("127.0.0.1:0").unwrap();
    let mut s = spec(&a);
    s.io_bind = taken.local_addr().unwrap();
    assert!(matches!(
        Scanner::open(&s),
        Err(EnipError::NotConnected { .. })
    ));
}

#[test]
fn inputs_come_from_the_adapter_alone_for_this_connection() {
    let a = Adapter::start(Script::default());
    let s = Scanner::open(&spec(&a)).unwrap();
    let opened = s.opened();
    assert!(wait(Duration::from_secs(1), || s.counters().consumed > 2));
    a.producing.store(false, Ordering::SeqCst);
    let before = s.inputs();
    let consumed = s.counters().consumed;
    let to = opened.receives_on;
    // Another host (127.0.0.2), another connection id, another size, no Class 1 frame at all.
    let other_host = UdpSocket::bind("127.0.0.2:0").unwrap();
    other_host
        .send_to(&input_frame(opened.t_to_o_id, 1, 900, None, &[7; 6]), to)
        .unwrap();
    a.io.send_to(
        &input_frame(opened.t_to_o_id ^ 1, 1, 901, None, &[7; 6]),
        to,
    )
    .unwrap();
    a.io.send_to(&input_frame(opened.t_to_o_id, 1, 902, None, &[7; 5]), to)
        .unwrap();
    a.io.send_to(&[1, 2, 3], to).unwrap();
    assert!(wait(Duration::from_secs(1), || s.counters().stray == 4));
    assert_eq!(s.inputs(), before, "none of them taken");
    assert_eq!(s.counters().consumed, consumed);
    // This connection's frames from the adapter: taken, a repeated count consumed but no update.
    a.io.send_to(&input_frame(opened.t_to_o_id, 1, 903, None, &[7; 6]), to)
        .unwrap();
    assert!(wait(Duration::from_secs(1), || s.inputs().data == [7; 6]));
    let updates = s.inputs().updates;
    a.io.send_to(&input_frame(opened.t_to_o_id, 2, 903, None, &[8; 6]), to)
        .unwrap();
    assert!(wait(Duration::from_secs(1), || s.counters().consumed
        == consumed + 2));
    assert_eq!(
        (s.inputs().data, s.inputs().updates),
        (vec![7; 6], updates),
        "the same count is no new data"
    );
    drop(s);
    // Inputs with a run/idle header, when the connection says so.
    let a = Adapter::start(Script {
        input_run_idle: true,
        ..Script::default()
    });
    let mut sp = spec(&a);
    sp.input_header = Header::RunIdle;
    let s = Scanner::open(&sp).unwrap();
    a.set_inputs(&[1; 6]);
    assert!(wait(Duration::from_secs(1), || s.inputs().run
        == Some(true)
        && s.inputs().data == [1; 6]));
    // An adapter whose reply names no I/O address is sent to at its own address, port 2222.
    let a = Adapter::start(Script {
        names_io: false,
        ..Script::default()
    });
    let s = Scanner::open(&spec(&a)).unwrap();
    assert_eq!(s.opened().sends_to, SocketAddr::new(a.addr.ip(), IO_PORT));
}

#[test]
fn a_silent_adapter_loses_the_connection_at_its_timeout() {
    let a = Adapter::start(Script::default());
    let mut sp = spec(&a);
    sp.timeout_multiplier = 1;
    let s = Scanner::open(&sp).unwrap();
    assert_eq!(s.opened().timeout, RPI * 8);
    let fo = &a.frames()[1];
    assert_eq!(
        items(&fo[24..])[1].1[6 + 18],
        1,
        "the multiplier the Forward_Open carries"
    );
    assert!(wait(Duration::from_secs(1), || s.counters().consumed > 2));
    a.producing.store(false, Ordering::SeqCst);
    let t = Instant::now();
    assert!(wait(Duration::from_secs(2), || s.state() != LinkState::Running));
    let took = t.elapsed();
    assert_eq!(s.state(), LinkState::Lost { timeout: RPI * 8 });
    assert!(
        took >= RPI * 7 && took < RPI * 8 + Duration::from_millis(200),
        "{took:?}"
    );
    // Counted where the frames leave, so none still on its way to the adapter is miscounted.
    let produced = s.counters().produced;
    thread::sleep(RPI * 4);
    assert_eq!(
        s.counters().produced,
        produced,
        "a lost connection produces nothing"
    );
    // Closing a lost connection asks nothing of the adapter.
    let frames = a.frames().len();
    s.close().unwrap();
    assert_eq!(a.frames().len(), frames);
}

#[test]
fn a_refused_close_says_so_and_a_drop_closes() {
    let a = Adapter::start(Script {
        close: Answer::Refuse(0x01, vec![0x0107]),
        ..Script::default()
    });
    let s = Scanner::open(&spec(&a)).unwrap();
    assert_eq!(
        s.close().unwrap_err(),
        EnipError::Close {
            general: 0x01,
            extended: vec![0x0107]
        }
    );
    assert!(unregistered(&a), "unregistered all the same");
    thread::sleep(RPI);
    let produced = a.outputs().len();
    thread::sleep(RPI * 3);
    assert_eq!(a.outputs().len(), produced);
    // A drop sends the Forward_Close and unregisters, bounded.
    let a = Adapter::start(Script::default());
    let s = Scanner::open(&spec(&a)).unwrap();
    let t = Instant::now();
    drop(s);
    assert!(t.elapsed() < Duration::from_millis(500));
    assert!(unregistered(&a));
    let commands: Vec<u16> = a.frames().iter().map(|f| le16(f, 0)).collect();
    assert_eq!(commands, [0x0065, 0x006F, 0x006F, 0x0066]);
    // A drop on an adapter that never answers the close is bounded by the response bound.
    let a = Adapter::start(Script {
        close: Answer::Silent,
        ..Script::default()
    });
    let s = Scanner::open(&spec_short(&a)).unwrap();
    let t = Instant::now();
    drop(s);
    assert!(
        t.elapsed() < Duration::from_millis(700),
        "{:?}",
        t.elapsed()
    );
    // ...and unregisters all the same, the close unanswered.
    assert!(
        unregistered(&a),
        "unregistered after a close with no answer"
    );
    let commands: Vec<u16> = a.frames().iter().map(|f| le16(f, 0)).collect();
    assert_eq!(commands, [0x0065, 0x006F, 0x006F, 0x0066]);
    let a = Adapter::start(Script {
        close: Answer::Silent,
        ..Script::default()
    });
    let s = Scanner::open(&spec_short(&a)).unwrap();
    assert_eq!(
        s.close().unwrap_err(),
        EnipError::NoAnswer {
            step: "Forward_Close",
            cause: "no answer within 300 ms".into()
        }
    );
    assert!(unregistered(&a));
}

/// The adapter is the address of its session and the one its reply names for the outputs; an
/// unspecified address in the reply is the session's.
#[test]
fn inputs_come_from_the_adapters_session_or_io_address_and_no_other() {
    let io2 = UdpSocket::bind("127.0.0.2:0").unwrap();
    let a = Adapter::start(Script {
        io_named: Some(io2.local_addr().unwrap()),
        ..Script::default()
    });
    a.producing.store(false, Ordering::SeqCst);
    let mut sp = spec(&a);
    sp.timeout_multiplier = 7;
    let s = Scanner::open(&sp).unwrap();
    // At multiplier 7 the timeout (20 ms × 512, 10.24 s) is longer than the standard's 10 s, and
    // the first input is awaited the longer.
    assert_eq!(s.opened().timeout, RPI * 512);
    assert_eq!(s.opened().first_input, RPI * 512);
    let fo = &a.frames()[1];
    assert_eq!(
        items(&fo[24..])[1].1[6 + 18],
        7,
        "the multiplier the Forward_Open carries"
    );
    let (id, to) = (s.opened().t_to_o_id, s.opened().receives_on);
    assert_eq!(s.opened().sends_to, io2.local_addr().unwrap());
    // From the session's address (127.0.0.1): taken.
    a.io.send_to(&input_frame(id, 1, 1, None, &[0xAA; 6]), to)
        .unwrap();
    assert!(wait(Duration::from_secs(1), || s.inputs().data == [0xAA; 6]));
    // From the address the reply named (127.0.0.2): taken.
    io2.send_to(&input_frame(id, 2, 2, None, &[0xBB; 6]), to)
        .unwrap();
    assert!(wait(Duration::from_secs(1), || s.inputs().data == [0xBB; 6]));
    // From a third (127.0.0.3): stray.
    let other = UdpSocket::bind("127.0.0.3:0").unwrap();
    other
        .send_to(&input_frame(id, 3, 3, None, &[0xCC; 6]), to)
        .unwrap();
    assert!(wait(Duration::from_secs(1), || s.counters().stray == 1));
    assert_eq!(s.inputs().data, [0xBB; 6]);
    // The outputs go where the reply named.
    io2.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let mut buf = [0u8; 64];
    assert!(io2.recv_from(&mut buf).is_ok());
    drop(s);
    // An unspecified address in the reply: the session's address, at the port named.
    let a = Adapter::start(Script {
        io_named: Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)),
        ..Script::default()
    });
    let s = Scanner::open(&spec(&a)).unwrap();
    assert_eq!(
        s.opened().sends_to,
        SocketAddr::new(a.addr.ip(), IO_PORT),
        "address and port both unspecified"
    );
    drop(s);
    let real = a.io.local_addr().unwrap();
    let a2 = Adapter::start(Script {
        io_named: Some(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            real.port(),
        )),
        ..Script::default()
    });
    let s = Scanner::open(&spec(&a2)).unwrap();
    assert_eq!(
        s.opened().sends_to,
        SocketAddr::new(a2.addr.ip(), real.port())
    );
}

/// The adapter named by the unspecified address is reached at the local host: its session's real
/// peer is the address its inputs are judged by, and the fallback for an unspecified address in
/// its reply (Linux: a connect to 0.0.0.0 reaches the local host).
#[cfg(target_os = "linux")]
#[test]
fn an_adapter_named_by_the_unspecified_address_is_judged_by_its_sessions_peer() {
    let a = Adapter::start(Script {
        io_named: Some(SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)),
        ..Script::default()
    });
    let mut sp = spec(&a);
    sp.adapter = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), a.addr.port());
    let s = Scanner::open(&sp).unwrap();
    assert_eq!(s.opened().session, a.addr, "the session reached 127.0.0.1");
    assert_eq!(
        s.opened().sends_to,
        SocketAddr::new(a.addr.ip(), IO_PORT),
        "the reply's 0.0.0.0 read as the session's"
    );
    a.set_inputs(&[0x42; 6]);
    assert!(wait(Duration::from_secs(1), || s.inputs().data == [0x42; 6]));
    assert!(wait(Duration::from_secs(1), || s.counters().consumed > 3));
    assert_eq!(s.counters().stray, 0);
    assert_eq!(s.state(), LinkState::Running);
}

/// Only this connection's inputs from the adapter keep it alive.
#[test]
fn stray_frames_never_keep_a_silent_adapters_connection() {
    let a = Adapter::start(Script::default());
    let s = Scanner::open(&spec(&a)).unwrap();
    let (id, to) = (s.opened().t_to_o_id, s.opened().receives_on);
    assert!(wait(Duration::from_secs(1), || s.counters().consumed > 2));
    a.producing.store(false, Ordering::SeqCst);
    let t = Instant::now();
    let other = UdpSocket::bind("127.0.0.2:0").unwrap();
    while s.state() == LinkState::Running && t.elapsed() < Duration::from_secs(1) {
        // Another host with this id, the adapter with another id: every 5 ms.
        other
            .send_to(&input_frame(id, 1, 1, None, &[1; 6]), to)
            .unwrap();
        a.io.send_to(&input_frame(id ^ 1, 1, 1, None, &[1; 6]), to)
            .unwrap();
        // This id at another size, and a datagram that is no Class 1 frame, from the adapter.
        a.io.send_to(&input_frame(id, 1, 1, None, &[1; 4]), to)
            .unwrap();
        a.io.send_to(&[1, 2, 3], to).unwrap();

        thread::sleep(Duration::from_millis(5));
    }
    let took = t.elapsed();
    assert_eq!(s.state(), LinkState::Lost { timeout: RPI * 4 });
    assert!(took < RPI * 4 + Duration::from_millis(200), "{took:?}");
    assert!(s.counters().stray > 10);
}

/// The timeout runs from the open: an adapter that never sends an input loses the connection.
#[test]
fn the_timeout_runs_from_the_open() {
    let a = Adapter::start(Script::default());
    a.producing.store(false, Ordering::SeqCst);
    // A first-input window shorter than the timeout: the timeout applies from the open.
    let mut sp = spec(&a);
    sp.first_input = Duration::ZERO;
    let before = Instant::now();
    let s = Scanner::open(&sp).unwrap();
    let after = Instant::now();
    assert_eq!(s.opened().first_input, RPI * 4);
    assert!(wait(Duration::from_secs(2), || s.state() != LinkState::Running));
    // Lost no sooner than the timeout after the open began, and soon after the open returned (a
    // stall of the machine during the open is not the scanner's).
    assert_eq!(s.state(), LinkState::Lost { timeout: RPI * 4 });
    let (least, most) = (before.elapsed(), after.elapsed());
    assert!(least >= RPI * 4, "{least:?}");
    assert!(most < RPI * 4 + Duration::from_millis(250), "{most:?}");
    assert_eq!(s.counters().consumed, 0);
    // The outputs went from the open on, before any input.
    assert!(
        s.counters().produced >= 2,
        "produced {}",
        s.counters().produced
    );
    assert!(
        a.outputs().len() >= 2,
        "the adapter received {}",
        a.outputs().len()
    );
}

/// The first input is awaited its window, the standard's 10 s by default, counted from the end of
/// the open: an open slower than the timeout loses nothing, and the window, not the timeout,
/// applies before the first input.
#[test]
fn the_first_input_is_awaited_its_window_from_the_end_of_the_open() {
    assert_eq!(
        Spec::new("127.0.0.1:44818".parse().unwrap(), 1, 2, 3, 0, 0).first_input,
        FIRST_INPUT
    );
    assert_eq!(FIRST_INPUT, Duration::from_secs(10));
    // An open that takes 300 ms against an 80 ms timeout and a 200 ms window, the adapter
    // producing from its reply: the connection runs.
    let a = Adapter::start(Script {
        open_delay: Duration::from_millis(300),
        ..Script::default()
    });
    let mut sp = spec(&a);
    sp.first_input = Duration::from_millis(200);
    let s = Scanner::open(&sp).unwrap();
    assert_eq!(s.opened().first_input, Duration::from_millis(200));
    thread::sleep(Duration::from_millis(500));
    assert_eq!(s.state(), LinkState::Running);
    assert!(
        s.counters().consumed >= 10,
        "consumed {}",
        s.counters().consumed
    );
    drop(s);
    // The same open, the adapter silent: lost at the window after the open returned.
    let a = Adapter::start(Script {
        open_delay: Duration::from_millis(300),
        ..Script::default()
    });
    a.producing.store(false, Ordering::SeqCst);
    let mut sp = spec(&a);
    sp.first_input = Duration::from_millis(200);
    let s = Scanner::open(&sp).unwrap();
    let after = Instant::now();
    assert!(wait(Duration::from_secs(2), || s.state() != LinkState::Running));
    let took = after.elapsed();
    assert_eq!(
        s.state(),
        LinkState::Lost {
            timeout: Duration::from_millis(200)
        }
    );
    assert!(
        took >= Duration::from_millis(150) && took < Duration::from_millis(450),
        "{took:?}"
    );
}

/// The first-input window is the longer of `first_input` and the timeout as the adapter's grant
/// makes it, and nothing but a first input of this connection ends it.
#[test]
fn the_first_input_window_is_the_longer_and_only_an_input_ends_it() {
    // A slower grant than asked (100 ms in), multiplier 1: the timeout is 800 ms, longer than a
    // 100 ms first-input window, so the window is 800 ms.
    let a = Adapter::start(Script {
        granted: Some((20_000, 100_000)),
        ..Script::default()
    });
    a.producing.store(false, Ordering::SeqCst);
    let mut sp = spec(&a);
    (sp.timeout_multiplier, sp.first_input) = (1, Duration::from_millis(100));
    let s = Scanner::open(&sp).unwrap();
    let after = Instant::now();
    assert_eq!(
        (s.opened().timeout, s.opened().first_input),
        (Duration::from_millis(800), Duration::from_millis(800))
    );
    assert!(wait(Duration::from_secs(3), || s.state() != LinkState::Running));
    let took = after.elapsed();
    assert_eq!(
        s.state(),
        LinkState::Lost {
            timeout: Duration::from_millis(800)
        }
    );
    assert!(
        took >= Duration::from_millis(750) && took < Duration::from_millis(1100),
        "{took:?}"
    );
    drop(s);
    // A 400 ms window, an 80 ms timeout, the adapter silent: strays of every kind for 200 ms
    // end nothing; a first input after the timeout but inside the window is taken, and from then
    // the timeout applies.
    let a = Adapter::start(Script::default());
    a.producing.store(false, Ordering::SeqCst);
    let mut sp = spec(&a);
    sp.first_input = Duration::from_millis(400);
    let s = Scanner::open(&sp).unwrap();
    let (id, to) = (s.opened().t_to_o_id, s.opened().receives_on);
    let other = UdpSocket::bind("127.0.0.2:0").unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(200) {
        other
            .send_to(&input_frame(id, 1, 1, None, &[1; 6]), to)
            .unwrap();
        a.io.send_to(&input_frame(id ^ 1, 1, 1, None, &[1; 6]), to)
            .unwrap();
        a.io.send_to(&input_frame(id, 1, 1, None, &[1; 4]), to)
            .unwrap();
        a.io.send_to(&[1, 2, 3], to).unwrap();
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        s.state(),
        LinkState::Running,
        "strays end no first-input window"
    );
    assert!(s.counters().stray > 20);
    a.io.send_to(&input_frame(id, 2, 7, None, &[0x77; 6]), to)
        .unwrap();
    assert!(wait(Duration::from_secs(1), || s.inputs().data == [0x77; 6]));
    assert_eq!(s.counters().consumed, 1);
    assert!(wait(Duration::from_secs(1), || s.state() != LinkState::Running));
    assert_eq!(
        s.state(),
        LinkState::Lost { timeout: RPI * 4 },
        "the timeout after an input"
    );
}

/// A caller waiting "forever" for the first input: the deadline saturates and the connection
/// runs and closes as any other.
#[test]
fn a_first_input_window_without_end_runs_and_closes() {
    let a = Adapter::start(Script::default());
    a.producing.store(false, Ordering::SeqCst);
    let mut sp = spec(&a);
    sp.first_input = Duration::MAX;
    let s = Scanner::open(&sp).unwrap();
    let opened = Instant::now();
    assert_eq!(s.opened().first_input, Duration::MAX);
    assert!(
        wait(Duration::from_secs(1), || a.outputs().len() > 3),
        "it produces"
    );
    // The adapter silent, past the 80 ms timeout and past the standard's 10 s: still running,
    // still producing. No fallback, neither the timeout nor `FIRST_INPUT`, stands in for "without
    // end".
    for past in [
        Duration::from_millis(300),
        FIRST_INPUT + Duration::from_millis(500),
    ] {
        let produced = s.counters().produced;
        thread::sleep(past.saturating_sub(opened.elapsed()));
        assert_eq!(s.state(), LinkState::Running, "at {past:?}");
        assert!(
            s.counters().produced > produced,
            "still producing at {past:?}"
        );
    }
    s.close().unwrap();
    assert!(unregistered(&a));
    let commands: Vec<u16> = a.frames().iter().map(|f| le16(f, 0)).collect();
    assert_eq!(
        commands,
        [0x0065, 0x006F, 0x006F, 0x0066],
        "the Forward_Close and the unregister went"
    );
}

/// A first input is new data whatever its count, 0 included.
#[test]
fn the_first_input_is_taken_whatever_its_count() {
    let a = Adapter::start(Script::default());
    a.producing.store(false, Ordering::SeqCst);
    let mut sp = spec(&a);
    sp.timeout_multiplier = 7;
    let s = Scanner::open(&sp).unwrap();
    let (id, to) = (s.opened().t_to_o_id, s.opened().receives_on);
    a.io.send_to(&input_frame(id, 1, 0, None, &[0x5A; 6]), to)
        .unwrap();
    assert!(wait(Duration::from_secs(1), || s.inputs().data == [0x5A; 6]));
    assert_eq!(s.inputs().updates, 1);
}

/// A flood of datagrams that are not this connection's holds nothing back: the outputs go at
/// their interval and the link stays up.
#[test]
fn a_flood_of_stray_datagrams_holds_no_output_back() {
    let a = Adapter::start(Script::default());
    let mut sp = spec(&a);
    sp.timeout_multiplier = 7;
    let s = Scanner::open(&sp).unwrap();
    assert!(wait(Duration::from_secs(1), || s.counters().consumed > 2));
    let to = s.opened().receives_on;
    let stop = Arc::new(AtomicBool::new(false));
    let flooders: Vec<_> = (0..2)
        .map(|_| {
            let stop = stop.clone();
            thread::spawn(move || {
                let u = UdpSocket::bind("127.0.0.2:0").unwrap();
                while !stop.load(Ordering::SeqCst) {
                    let _ = u.send_to(&[1, 2, 3], to);
                }
            })
        })
        .collect();
    thread::sleep(Duration::from_millis(100));
    let (p0, a0) = (s.counters().produced, a.outputs().len());
    thread::sleep(Duration::from_secs(1));
    let (p1, a1) = (s.counters().produced, a.outputs().len());
    stop.store(true, Ordering::SeqCst);
    for f in flooders {
        f.join().unwrap();
    }
    assert!(
        s.counters().stray > 1000,
        "the flood reached the scanner: {}",
        s.counters().stray
    );
    assert!(
        (40..=60).contains(&(p1 - p0)),
        "produced {} during the flood",
        p1 - p0
    );
    assert!(
        (40..=60).contains(&(a1 - a0)),
        "the adapter received {} during the flood",
        a1 - a0
    );
    assert_eq!(s.state(), LinkState::Running);
}

/// Each direction at the interval the adapter granted it: outputs at O→T's, the timeout from
/// T→O's.
#[test]
fn each_direction_runs_at_its_own_granted_interval() {
    let a = Adapter::start(Script {
        granted: Some((10_000, 100_000)),
        ..Script::default()
    });
    let s = Scanner::open(&spec(&a)).unwrap();
    let o = s.opened();
    assert_eq!(
        (o.o_to_t_rpi, o.t_to_o_rpi),
        (Duration::from_millis(10), Duration::from_millis(100))
    );
    assert_eq!(o.timeout, Duration::from_millis(400));
    let p0 = s.counters().produced;
    thread::sleep(Duration::from_secs(1));
    let produced = s.counters().produced - p0;
    assert!(
        (80..=110).contains(&produced),
        "produced {produced} at 10 ms"
    );
    assert_eq!(
        s.state(),
        LinkState::Running,
        "inputs every 100 ms keep a 400 ms timeout"
    );
    assert!(
        (7..=11).contains(&s.counters().consumed),
        "consumed {}",
        s.counters().consumed
    );
    // A granted O→T interval of 0 is refused as a T→O one is.
    let a = Adapter::start(Script {
        granted: Some((0, 20_000)),
        ..Script::default()
    });
    assert_eq!(
        Scanner::open(&spec(&a)).unwrap_err(),
        EnipError::NoAnswer {
            step: "Forward_Open",
            cause: "a granted interval of 0".into()
        }
    );
}

/// Outputs without a header carry the count and the data alone; inputs with one say idle when
/// the adapter is idle.
#[test]
fn modeless_outputs_and_idle_inputs() {
    let a = Adapter::start(Script {
        input_run_idle: true,
        ..Script::default()
    });
    let mut sp = spec(&a);
    (sp.output_header, sp.input_header) = (Header::Modeless, Header::RunIdle);
    let s = Scanner::open(&sp).unwrap();
    // The Forward_Open's sizes: 4 + 2 out, 6 + 2 + 4 in.
    let fo = &a.frames()[1];
    let body = &items(&fo[24..])[1].1[6..];
    assert_eq!(le16(body, 26) & 0x01FF, 6);
    assert_eq!(le16(body, 32) & 0x01FF, 12);
    s.set_outputs(&[5, 6, 7, 8]).unwrap();
    assert!(wait(Duration::from_secs(1), || a
        .outputs()
        .iter()
        .any(|f| f.ends_with(&[5, 6, 7, 8]))));
    let f = a
        .outputs()
        .into_iter()
        .find(|f| f.ends_with(&[5, 6, 7, 8]))
        .unwrap();
    assert_eq!(
        &f[14..18],
        [0xB1, 0x00, 6, 0],
        "the count and 4 bytes, no header"
    );
    assert_eq!(f.len(), 24);
    // The adapter idle: the inputs say so.
    a.producing.store(false, Ordering::SeqCst);
    let (id, to) = (s.opened().t_to_o_id, s.opened().receives_on);
    a.io.send_to(&input_frame(id, 50, 500, Some(false), &[3; 6]), to)
        .unwrap();
    assert!(wait(Duration::from_secs(1), || s.inputs().run == Some(false)));
    a.io.send_to(&input_frame(id, 51, 501, Some(true), &[4; 6]), to)
        .unwrap();
    assert!(wait(Duration::from_secs(1), || s.inputs().run
        == Some(true)
        && s.inputs().data == [4; 6]));
}

// ---------------------------------------------------------------------------------------------
// The dissectors: Wireshark's reading of the standard (`tshark`), outside this crate.
// ---------------------------------------------------------------------------------------------

/// A capture of `segments` (true: from the scanner) as one TCP conversation to port 44818, and
/// `datagrams` as UDP to and from port 2222.
fn capture(segments: &[(bool, Vec<u8>)], datagrams: &[(bool, Vec<u8>)]) -> Vec<Vec<u8>> {
    fn sum(b: &[u8]) -> u16 {
        let mut s: u32 = b
            .chunks(2)
            .map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32)
            .sum();
        while s > 0xFFFF {
            s = (s & 0xFFFF) + (s >> 16);
        }
        !(s as u16)
    }
    fn ip(src: [u8; 4], dst: [u8; 4], proto: u8, payload: &[u8]) -> Vec<u8> {
        let mut h = vec![0x45, 0, 0, 0, 0, 1, 0, 0, 64, proto, 0, 0];
        h[2..4].copy_from_slice(&((20 + payload.len()) as u16).to_be_bytes());
        h.extend_from_slice(&src);
        h.extend_from_slice(&dst);
        let c = sum(&h);
        h[10..12].copy_from_slice(&c.to_be_bytes());
        let mut f = vec![0x02, 0, 0, 0, 0, 2, 0x02, 0, 0, 0, 0, 1, 0x08, 0x00];
        f.extend(h);
        f.extend_from_slice(payload);
        f
    }
    fn tcp(
        ends: (([u8; 4], u16), ([u8; 4], u16)),
        seq: u32,
        ack: u32,
        flags: u8,
        data: &[u8],
    ) -> Vec<u8> {
        let ((src, sp), (dst, dp)) = ends;
        let mut t = Vec::new();
        t.extend_from_slice(&sp.to_be_bytes());
        t.extend_from_slice(&dp.to_be_bytes());
        t.extend_from_slice(&seq.to_be_bytes());
        t.extend_from_slice(&ack.to_be_bytes());
        t.extend_from_slice(&[0x50, flags, 0xFF, 0xFF, 0, 0, 0, 0]);
        t.extend_from_slice(data);
        let mut pseudo = Vec::new();
        pseudo.extend_from_slice(&src);
        pseudo.extend_from_slice(&dst);
        pseudo.extend_from_slice(&[0, 6]);
        pseudo.extend_from_slice(&(t.len() as u16).to_be_bytes());
        pseudo.extend_from_slice(&t);
        let c = sum(&pseudo);
        t[16..18].copy_from_slice(&c.to_be_bytes());
        ip(src, dst, 6, &t)
    }
    let (a, b) = ([10, 0, 0, 1], [10, 0, 0, 2]);
    let (mut sa, mut sb) = (1000u32, 5000u32);
    let (o, t) = ((a, 40000), (b, 44818));
    let mut out = vec![
        tcp((o, t), sa - 1, 0, 0x02, &[]),
        tcp((t, o), sb - 1, sa, 0x12, &[]),
        tcp((o, t), sa, sb, 0x10, &[]),
    ];
    for (mine, data) in segments {
        if *mine {
            out.push(tcp((o, t), sa, sb, 0x18, data));
            sa += data.len() as u32;
        } else {
            out.push(tcp((t, o), sb, sa, 0x18, data));
            sb += data.len() as u32;
        }
    }
    for (mine, data) in datagrams {
        let (src, dst) = if *mine { (a, b) } else { (b, a) };
        let mut u = Vec::new();
        u.extend_from_slice(&2222u16.to_be_bytes());
        u.extend_from_slice(&2222u16.to_be_bytes());
        u.extend_from_slice(&((8 + data.len()) as u16).to_be_bytes());
        u.extend_from_slice(&[0, 0]);
        u.extend_from_slice(data);
        out.push(ip(src, dst, 17, &u));
    }
    out
}

fn tshark() -> PathBuf {
    let candidates: Vec<PathBuf> = [
        std::env::var_os("VEREDAS_TSHARK").map(PathBuf::from),
        std::env::var_os("HOME").map(|h| Path::new(&h).join(".local/opt/wireshark/tshark")),
        Some(PathBuf::from("tshark")),
    ]
    .into_iter()
    .flatten()
    .collect();
    for c in &candidates {
        if Command::new(c)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return c.clone();
        }
    }
    panic!(
        "the dissectors are this test's reading of the standard apart from the scanner, and no tshark runs here: tried {candidates:?}; set VEREDAS_TSHARK"
    );
}

/// Every packet's layers as the dissectors read them (`-T ek`).
fn dissect(frames: &[Vec<u8>]) -> Vec<Json> {
    let dir = std::env::temp_dir().join(format!("veredas-core-enip-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("conversation.pcap");
    let mut pcap = Vec::new();
    pcap.extend_from_slice(&0xa1b2_c3d4u32.to_le_bytes());
    pcap.extend_from_slice(&2u16.to_le_bytes());
    pcap.extend_from_slice(&4u16.to_le_bytes());
    pcap.extend_from_slice(&[0; 8]);
    pcap.extend_from_slice(&65535u32.to_le_bytes());
    pcap.extend_from_slice(&1u32.to_le_bytes());
    for (i, f) in frames.iter().enumerate() {
        pcap.extend_from_slice(&(1_700_000_000u32 + i as u32).to_le_bytes());
        pcap.extend_from_slice(&0u32.to_le_bytes());
        pcap.extend_from_slice(&(f.len() as u32).to_le_bytes());
        pcap.extend_from_slice(&(f.len() as u32).to_le_bytes());
        pcap.extend_from_slice(f);
    }
    std::fs::write(&file, pcap).unwrap();
    let out = Command::new(tshark())
        .arg("-n")
        .arg("-r")
        .arg(&file)
        .args(["-T", "ek"])
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(
        out.status.success(),
        "tshark: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .filter(|l| !l.starts_with("{\"index\""))
        .map(|l| serde_json::from_str::<Json>(l).unwrap()["layers"].clone())
        .collect()
}

fn values(v: &Json, key: &str, out: &mut Vec<String>) {
    match v {
        Json::Object(map) => {
            for (k, x) in map {
                if k == key {
                    match x {
                        Json::Array(items) => out.extend(items.iter().map(|i| {
                            i.as_str()
                                .map(String::from)
                                .unwrap_or_else(|| i.to_string())
                        })),
                        other => out.push(
                            other
                                .as_str()
                                .map(String::from)
                                .unwrap_or_else(|| other.to_string()),
                        ),
                    }
                } else {
                    values(x, key, out);
                }
            }
        }
        Json::Array(items) => items.iter().for_each(|x| values(x, key, out)),
        _ => {}
    }
}

fn field(layers: &Json, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    values(layers, key, &mut out);
    out
}

/// What the dissectors complained of: a malformed packet, or expert information at warning or
/// error severity.
fn complaints(layers: &Json) -> Vec<String> {
    let mut out = Vec::new();
    if !layers["_ws_malformed"].is_null() {
        out.push(format!("malformed: {}", layers["_ws_malformed"]));
    }
    for sev in field(layers, "_ws_expert_severity") {
        if sev.parse::<u64>().unwrap_or(0) >= 6_291_456 {
            out.push(format!("expert at severity {sev}: {layers}"));
        }
    }
    out
}
