//! The EtherNet/IP adapter (contract: the EtherNet/IP adapter clause): a device's assemblies on a
//! real TCP session and real Class 1 UDP frames, met by raw encapsulation bytes; by an originator
//! built from codecs the bench did not write (`ethernetip-connections`' Forward_Open and Class 1
//! frame, `ethernetip-core`'s encapsulation and common packet format, with the protocol's sizes and
//! item codes); and read back by Wireshark's dissectors (`tshark`), the second reading of the
//! standard. Each refusal comes with the status the protocol gives it; the plant side by point and
//! by bytes; the knobs; the ports freed on stop.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use ethernetip_connections::{ForwardCloseRequest, ForwardOpenRequest, ForwardOpenResponse, NetworkConnectionParameters};
use serde_json::Value as Json;
use veredas_maquinar::ethernetip::*;

fn config(text: &str) -> EnipConfig {
    toml::from_str(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
}

fn refused(text: &str) -> String {
    match validate(&config(text)) {
        Ok(()) => panic!("accepted: {text}"),
        Err(e) => e.to_string(),
    }
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

/// A device on a loopback address of its own, so its ports are its own in a parallel run.
fn device(host: u8) -> String {
    format!(
        "bind = \"127.0.0.{host}\"\nport = 0\nio_port = 0\n\
         identity = {{ vendor_id = 9, device_type = 7, product_code = 3, revision = \"2.5\", serial = 305419896, product_name = \"bench io\" }}\n\
         assemblies = [ {{ instance = 100, size = 8 }}, {{ instance = 150, size = 4 }}, {{ instance = 151, size = 2 }} ]\n\
         connections = [ {{ config = 151, output = 150, input = 100 }} ]\n\
         points = [\n\
           {{ name = \"start\", assembly = 150, byte = 0, bit = 0 }},\n\
           {{ name = \"setpoint\", assembly = 150, byte = 2, type = \"u16\" }},\n\
           {{ name = \"running\", assembly = 100, byte = 0, bit = 1 }},\n\
           {{ name = \"speed\", assembly = 100, byte = 4, type = \"f32\", value = \"12.5\" }},\n\
         ]\n"
    )
}

/// A raw encapsulation client: what is on the wire is what it asserts.
struct Encap {
    stream: TcpStream,
    /// Every frame sent and received, in order, for the dissectors.
    log: Vec<(bool, Vec<u8>)>,
}

impl Encap {
    fn connect(addr: SocketAddr) -> Encap {
        let stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream.set_nodelay(true).unwrap();
        Encap { stream, log: Vec::new() }
    }

    fn frame(command: u16, session: u32, context: [u8; 8], body: &[u8]) -> Vec<u8> {
        let mut buf = bytes::BytesMut::new();
        ethernetip_core::encap::Header { command, length: body.len() as u16, session_handle: session, status: 0, sender_context: context, options: 0 }
            .write_to(&mut buf);
        buf.extend_from_slice(body);
        buf.to_vec()
    }

    fn send(&mut self, bytes: &[u8]) {
        self.log.push((true, bytes.to_vec()));
        self.stream.write_all(bytes).unwrap();
    }

    fn read(&mut self) -> (ethernetip_core::encap::Header, Vec<u8>) {
        let mut h = [0u8; 24];
        self.stream.read_exact(&mut h).unwrap();
        let header = ethernetip_core::encap::Header::parse(&h).unwrap();
        let mut b = vec![0u8; header.length as usize];
        self.stream.read_exact(&mut b).unwrap();
        self.log.push((false, [h.to_vec(), b.clone()].concat()));
        (header, b)
    }

    fn ask(&mut self, command: u16, session: u32, body: &[u8]) -> (ethernetip_core::encap::Header, Vec<u8>) {
        self.send(&Encap::frame(command, session, *b"bench\x01\x02\x03", body));
        self.read()
    }

    fn register(&mut self) -> u32 {
        let (h, _) = self.ask(0x65, 0, &[1, 0, 0, 0]);
        assert_eq!(h.status, 0);
        assert_ne!(h.session_handle, 0);
        h.session_handle
    }

    /// A CIP request sent unconnected with these items besides; the reply's envelope.
    fn rr(&mut self, session: u32, request: &[u8], extra: Vec<ethernetip_core::cpf::Item>) -> ethernetip_core::cpf::Envelope {
        let mut items = vec![ethernetip_core::cpf::Item::null_address(), ethernetip_core::cpf::Item::new(0x00B2, request.to_vec())];
        items.extend(extra);
        let body = ethernetip_core::cpf::encode_envelope(0, 5, &items);
        let (h, reply) = self.ask(0x6F, session, &body);
        assert_eq!(h.status, 0, "SendRRData status");
        ethernetip_core::cpf::Envelope::parse(&reply).unwrap()
    }

    /// A CIP request; its reply frame.
    fn cip(&mut self, session: u32, request: &[u8]) -> Vec<u8> {
        let env = self.rr(session, request, Vec::new());
        env.find(0x00B2).unwrap().data.clone()
    }
}

/// The originator's side of a Class 1 connection, from the independent codecs: the Forward_Open
/// with the protocol's sizes (the data, the 16-bit sequence count, the 32-bit run/idle header when
/// the direction has one), a socket for the produced frames, the frames it sends.
struct Originator {
    encap: Encap,
    session: u32,
    udp: UdpSocket,
    serial: u16,
    t_to_o_id: u32,
}

/// A Forward_Open's fields a test varies.
#[derive(Clone)]
struct Open {
    config: u16,
    output: u16,
    input: u16,
    o_to_t_size: u16,
    t_to_o_size: u16,
    rpi_us: u32,
    multiplier: u8,
    transport: u8,
    o_to_t_kind: u8,
    t_to_o_kind: u8,
    key: Option<[u8; 10]>,
    config_data: Option<Vec<u8>>,
    serial: u16,
}

fn open_for(output_data: u16, input_data: u16) -> Open {
    Open {
        config: 151,
        output: 150,
        input: 100,
        // Run/idle on the consumed side, modeless on the produced one (the defaults).
        o_to_t_size: output_data + 2 + 4,
        t_to_o_size: input_data + 2,
        rpi_us: 20_000,
        multiplier: 2,
        transport: 0x01,
        o_to_t_kind: 2,
        t_to_o_kind: 2,
        key: None,
        config_data: None,
        serial: 0x1234,
    }
}

impl Originator {
    fn connect(adapter: &EnipHandle) -> Originator {
        let mut encap = Encap::connect(adapter.addr);
        let session = encap.register();
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        udp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        Originator { encap, session, udp, serial: 0, t_to_o_id: 0 }
    }

    fn request(&self, o: &Open) -> ForwardOpenRequest {
        let mut path = Vec::new();
        if let Some(key) = o.key {
            path.extend_from_slice(&key);
        }
        path.extend_from_slice(&[0x20, 0x04]);
        path.extend_from_slice(&[0x24, o.config as u8]);
        if let Some(data) = &o.config_data {
            path.extend_from_slice(&[0x80, data.len().div_ceil(2) as u8]);
            path.extend_from_slice(data);
            if data.len() % 2 == 1 {
                path.push(0);
            }
        }
        path.extend_from_slice(&[0x2C, o.output as u8, 0x2C, o.input as u8]);
        let params = |kind: u8, size: u16| NetworkConnectionParameters { redundant_owner: false, connection_type: kind, priority: 0, variable_size: false, size };
        ForwardOpenRequest {
            priority_tick: 0x0A,
            timeout_ticks: 0x05,
            o_to_t_connection_id: 0,
            t_to_o_connection_id: 0x8000_0000 | o.serial as u32,
            connection_serial: o.serial,
            originator_vendor: 0x00FE,
            originator_serial: 0x0BADCAFE,
            connection_timeout_mult: o.multiplier,
            o_to_t_rpi_us: o.rpi_us,
            o_to_t_params: params(o.o_to_t_kind, o.o_to_t_size),
            t_to_o_rpi_us: o.rpi_us,
            t_to_o_params: params(o.t_to_o_kind, o.t_to_o_size),
            transport_type: o.transport,
            connection_path: path,
        }
    }

    /// Send a Forward_Open (through the Connection Manager, the originator's T→O endpoint in the
    /// protocol's item 0x8001); the reply's frame and envelope.
    fn forward_open(&mut self, o: &Open) -> (Vec<u8>, ethernetip_core::cpf::Envelope) {
        let mut mr = vec![0x54, 0x02, 0x20, 0x06, 0x24, 0x01];
        mr.extend(self.request(o).encode());
        let SocketAddr::V4(me) = self.udp.local_addr().unwrap() else { panic!() };
        let sockaddr = ethernetip_core::cpf::encode_sockaddr_in_v4(SocketAddr::V4(me)).unwrap();
        let env = self.encap.rr(self.session, &mr, vec![ethernetip_core::cpf::Item::new(0x8001, sockaddr)]);
        let reply = env.find(0x00B2).unwrap().data.clone();
        if reply[2] == 0 {
            self.serial = o.serial;
            self.t_to_o_id = 0x8000_0000 | o.serial as u32;
        }
        (reply, env)
    }

    /// A Forward_Open that must succeed: its decoded reply and where the adapter consumes.
    fn open(&mut self, adapter: &EnipHandle, o: &Open) -> (ForwardOpenResponse, SocketAddr) {
        let (reply, env) = self.forward_open(o);
        assert_eq!(&reply[..4], &[0xD4, 0x00, 0x00, 0x00], "Forward_Open refused: {reply:02X?}");
        let response = ForwardOpenResponse::decode(&reply[4..]).unwrap();
        // The adapter's I/O port is not the standard one here: its reply says where it is.
        let to = env.find(0x8000).map(|i| SocketAddr::V4(ethernetip_core::cpf::decode_sockaddr_in_v4(&i.data).unwrap())).unwrap();
        assert_eq!(to, adapter.io_addr);
        (response, to)
    }

    fn forward_close(&mut self, serial: u16) -> Vec<u8> {
        let mut path = vec![0x20, 0x04, 0x24, 151, 0x2C, 150, 0x2C, 100];
        let fc = ForwardCloseRequest { priority_tick: 0x0A, timeout_ticks: 0x05, connection_serial: serial, originator_vendor: 0x00FE, originator_serial: 0x0BADCAFE, connection_path: std::mem::take(&mut path) };
        let mut mr = vec![0x4E, 0x02, 0x20, 0x06, 0x24, 0x01];
        mr.extend(fc.encode());
        self.encap.cip(self.session, &mr)
    }

    /// The next produced frame, read by the independent codec.
    fn next_input(&self, run_idle: bool) -> ethernetip_connections::Frame {
        let mut buf = [0u8; 1500];
        let (n, _) = self.udp.recv_from(&mut buf).unwrap();
        ethernetip_connections::decode_frame(&buf[..n], run_idle).unwrap()
    }

    fn send_output(&self, to: SocketAddr, connection: u32, sequence: u32, count: u16, run: bool, data: &[u8]) {
        let frame = ethernetip_connections::encode_frame(&ethernetip_connections::Frame {
            connection_id: connection,
            sequence,
            cip_sequence: count,
            run_idle: Some(run),
            data: data.to_vec(),
        });
        self.udp.send_to(&frame, to).unwrap();
    }
}

/// The general status and the additional words of a reply frame.
fn status_of(reply: &[u8]) -> (u8, Vec<u16>) {
    let n = reply[3] as usize;
    (reply[2], (0..n).map(|i| u16::from_le_bytes([reply[4 + 2 * i], reply[5 + 2 * i]])).collect())
}

#[test]
fn the_configuration_reads_from_toml_with_its_defaults_and_refuses_what_cannot_serve() {
    let c = config("");
    assert_eq!(c, EnipConfig::default());
    assert_eq!((c.bind.as_str(), c.port, c.io_port, c.rpi_min_us, c.rpi_max_us), ("127.0.0.1", 44818, 2222, 1_000, 10_000_000));
    assert_eq!(c.identity, Identity { vendor_id: 0, device_type: 12, product_code: 0, revision: "1.1".into(), serial: 0, product_name: "veredas bench adapter".into() });
    let e = toml::from_str::<EnipConfig>("prot = 1").unwrap_err().to_string();
    assert!(e.contains("unknown field"), "{e}");
    let base = |rest: &str| format!("assemblies = [ {{ instance = 100, size = 8 }}, {{ instance = 150, size = 4 }}, {{ instance = 151, size = 0 }} ]\n{rest}");
    assert_eq!(refused("bind = \" \""), "ethernetip.bind is empty");
    assert_eq!(refused("identity = { revision = \"1\" }"), "ethernetip.identity.revision: `1` is not major.minor");
    assert_eq!(refused("identity = { revision = \"0.1\" }"), "ethernetip.identity.revision: `0.1`: a major revision is 1 to 127 and a minor 1 to 255");
    assert_eq!(refused("identity = { product_name = \"an adapter whose name runs past 32\" }"), "ethernetip.identity.product_name: at most 32 ASCII characters");
    assert_eq!(refused("rpi_min_us = 2000\nrpi_max_us = 1000"), "ethernetip: the RPI range 2000..=1000 us is empty");
    assert_eq!(refused("max_connections = 0"), "ethernetip: a limit of 0 lets nobody in");
    assert_eq!(refused("assemblies = [ { instance = 0, size = 1 } ]"), "ethernetip.assemblies[0]: instance 0 is the class, not an assembly");
    assert_eq!(refused("assemblies = [ { instance = 5, size = 1 }, { instance = 5, size = 2 } ]"), "ethernetip.assemblies[1]: instance 5 is declared twice");
    assert_eq!(refused(&base("connections = [ { config = 151, output = 150, input = 101 } ]")), "ethernetip.connections[0].input: no assembly 101");
    assert_eq!(refused(&base("connections = [ { config = 151, output = 150, input = 150 } ]")), "ethernetip.connections[0]: assembly 150 cannot be both consumed and produced");
    assert_eq!(refused(&base("points = [ { name = \"a\", assembly = 100, byte = 0, type = \"bool\" } ]")), "ethernetip.points[0] (`a`): a bool names its bit");
    assert_eq!(refused(&base("points = [ { name = \"a\", assembly = 100, byte = 0, bit = 8 } ]")), "ethernetip.points[0] (`a`): bit 8 is not 0 to 7");
    assert_eq!(refused(&base("points = [ { name = \"a\", assembly = 100, byte = 0, bit = 1, type = \"u8\" } ]")), "ethernetip.points[0] (`a`): only a bool takes a bit");
    assert_eq!(refused(&base("points = [ { name = \"a\", assembly = 100, byte = 6, type = \"f32\" } ]")), "ethernetip.points[0] (`a`): a f32 at byte 6 runs past assembly 100's 8 byte(s)");
    assert_eq!(refused(&base("points = [ { name = \"a\", assembly = 100, byte = 0, type = \"u16\" }, { name = \"b\", assembly = 100, byte = 1 } ]")), "ethernetip.points[1] (`b`): it overlaps point `a`");
    assert_eq!(refused(&base("points = [ { name = \"a\", assembly = 100, byte = 0, bit = 3 }, { name = \"b\", assembly = 100, byte = 0, bit = 3 } ]")), "ethernetip.points[1] (`b`): it overlaps point `a`");
    assert!(validate(&config(&base("points = [ { name = \"a\", assembly = 100, byte = 0, bit = 3 }, { name = \"b\", assembly = 100, byte = 0, bit = 4 } ]"))).is_ok());
    assert_eq!(refused(&base("points = [ { name = \"a\", assembly = 100, byte = 0, type = \"i8\", value = \"200\" } ]")), "ethernetip.points[0] (`a`).value: `200` is not a i8");
}

/// The identity item a ListIdentity answers with, laid out as the protocol has it.
fn identity_item(addr: SocketAddr, status: u16) -> Vec<u8> {
    let SocketAddr::V4(v4) = addr else { panic!() };
    let mut body = vec![1, 0];
    body.extend_from_slice(&2u16.to_be_bytes());
    body.extend_from_slice(&v4.port().to_be_bytes());
    body.extend_from_slice(&v4.ip().octets());
    body.extend_from_slice(&[0; 8]);
    body.extend_from_slice(&9u16.to_le_bytes());
    body.extend_from_slice(&7u16.to_le_bytes());
    body.extend_from_slice(&3u16.to_le_bytes());
    body.extend_from_slice(&[2, 5]);
    body.extend_from_slice(&status.to_le_bytes());
    body.extend_from_slice(&305419896u32.to_le_bytes());
    body.push(8);
    body.extend_from_slice(b"bench io");
    body.push(3);
    let mut out = vec![1, 0, 0x0C, 0x00];
    out.extend_from_slice(&(body.len() as u16).to_le_bytes());
    out.extend(body);
    out
}

#[test]
fn list_identity_answers_on_tcp_and_udp_with_the_configured_identity() {
    let adapter = start(&config(&device(41))).unwrap();
    let mut c = Encap::connect(adapter.addr);
    let (h, body) = c.ask(0x63, 0, &[]);
    assert_eq!((h.command, h.session_handle, h.status, h.sender_context, h.options), (0x63, 0, 0, *b"bench\x01\x02\x03", 0));
    assert_eq!(body, identity_item(adapter.addr, 0));
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    udp.send_to(&Encap::frame(0x63, 0, *b"udpquery", &[]), adapter.addr).unwrap();
    let mut buf = [0u8; 600];
    let (n, from) = udp.recv_from(&mut buf).unwrap();
    assert_eq!(from, adapter.addr);
    assert_eq!(&buf[12..20], b"udpquery");
    assert_eq!(buf[24..n].to_vec(), identity_item(adapter.addr, 0));
    // ListServices: one service, CIP over TCP and Class 1 over UDP; ListInterfaces: none.
    let (h, body) = c.ask(0x04, 0, &[]);
    assert_eq!(h.status, 0);
    let mut services = vec![1, 0, 0x00, 0x01, 20, 0, 1, 0, 0x20, 0x01];
    services.extend_from_slice(b"Communications\0\0");
    assert_eq!(body, services);
    let (h, body) = c.ask(0x64, 0, &[]);
    assert_eq!((h.status, body), (0, vec![0, 0]));
}

#[test]
fn a_session_is_registered_once_and_held_to_its_handle() {
    let adapter = start(&config(&format!("{}max_sessions = 1\n", device(42)))).unwrap();
    let mut c = Encap::connect(adapter.addr);
    // A request before any session: refused, the connection kept.
    let (h, _) = c.ask(0x6F, 0, &[0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(h.status, 0x64);
    // A version the adapter does not speak: refused with the one it does.
    let (h, body) = c.ask(0x65, 0, &[2, 0, 0, 0]);
    assert_eq!((h.status, h.session_handle, body), (0x69, 0, vec![1, 0, 0, 0]));
    let (h, body) = c.ask(0x65, 0, &[1, 0]);
    assert_eq!((h.status, body), (0x65, vec![1, 0]));
    let session = c.register();
    let (h, _) = c.ask(0x65, 0, &[1, 0, 0, 0]);
    assert_eq!((h.status, h.session_handle), (0x01, session), "a second registration on one connection");
    // Another handle than the session's: refused.
    let (h, _) = c.ask(0x6F, session + 1, &[0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(h.status, 0x64);
    // A command the adapter does not take (connected explicit messaging among them): 0x0001.
    for command in [0x0070u16, 0x0001, 0x00AA] {
        let (h, body) = c.ask(command, session, &[]);
        assert_eq!((h.command, h.status, body.len()), (command, 0x0001, 0), "command {command:#06x}");
    }
    // A SendRRData whose items are not a null address and unconnected data: incorrect data.
    let body = ethernetip_core::cpf::encode_envelope(0, 5, &[ethernetip_core::cpf::Item::new(0x00B2, vec![0x0E, 0x00])]);
    let (h, _) = c.ask(0x6F, session, &body);
    assert_eq!(h.status, 0x0003);
    let (h, _) = c.ask(0x6F, session, &[0, 0, 0, 0, 5, 0, 9, 0]);
    assert_eq!(h.status, 0x0003, "an item count past the bytes");
    // NOP: no answer; the next request is answered as the next.
    c.send(&Encap::frame(0x00, session, [0; 8], &[1, 2, 3]));
    let (h, _) = c.ask(0x63, 0, &[]);
    assert_eq!(h.command, 0x63);
    // The limit: a second connection may not register while the first holds its session.
    let mut d = Encap::connect(adapter.addr);
    let (h, _) = d.ask(0x65, 0, &[1, 0, 0, 0]);
    assert_eq!(h.status, 0x0002);
    // UnRegisterSession ends the connection, and frees the place.
    c.send(&Encap::frame(0x66, session, [0; 8], &[]));
    let mut b = [0u8; 1];
    assert_eq!(c.stream.read(&mut b).unwrap(), 0);
    assert!(wait(Duration::from_secs(5), || adapter.status().sessions == 0));
    let (h, _) = d.ask(0x65, 0, &[1, 0, 0, 0]);
    assert_eq!(h.status, 0);
    let s = adapter.status();
    assert_eq!((s.sessions, s.commands.get(&0x0065).copied()), (1, Some(6)));
}

#[test]
fn the_identity_object_answers_its_attributes_and_refuses_the_rest() {
    let adapter = start(&config(&device(43))).unwrap();
    let mut c = Encap::connect(adapter.addr);
    let s = c.register();
    let all = c.cip(s, &[0x01, 0x02, 0x20, 0x01, 0x24, 0x01]);
    let mut expected = vec![0x81, 0, 0, 0];
    expected.extend_from_slice(&[9, 0, 7, 0, 3, 0, 2, 5, 0, 0, 0x78, 0x56, 0x34, 0x12, 8]);
    expected.extend_from_slice(b"bench io");
    assert_eq!(all, expected);
    let single = |c: &mut Encap, attribute: u8| c.cip(s, &[0x0E, 0x03, 0x20, 0x01, 0x24, 0x01, 0x30, attribute]);
    let values: [&[u8]; 7] = [&[9, 0], &[7, 0], &[3, 0], &[2, 5], &[0, 0], &[0x78, 0x56, 0x34, 0x12], b"\x08bench io"];
    for (i, v) in values.iter().enumerate() {
        let r = single(&mut c, i as u8 + 1);
        assert_eq!(&r[..4], &[0x8E, 0, 0, 0], "attribute {}", i + 1);
        assert_eq!(&r[4..], *v, "attribute {}", i + 1);
    }
    assert_eq!(status_of(&single(&mut c, 8)), (0x14, vec![]));
    assert_eq!(status_of(&c.cip(s, &[0x10, 0x03, 0x20, 0x01, 0x24, 0x01, 0x30, 0x01, 0x00, 0x00])), (0x0E, vec![]), "the vendor is not settable");
    assert_eq!(status_of(&c.cip(s, &[0x0E, 0x03, 0x20, 0x01, 0x24, 0x00, 0x30, 0x01])), (0x00, vec![]), "the class's revision");
    assert_eq!(status_of(&c.cip(s, &[0x0E, 0x03, 0x20, 0x01, 0x24, 0x02, 0x30, 0x01])), (0x05, vec![]), "no second instance");
    assert_eq!(status_of(&c.cip(s, &[0x4B, 0x02, 0x20, 0x01, 0x24, 0x01])), (0x08, vec![]), "a service it does not offer");
    assert_eq!(status_of(&c.cip(s, &[0x0E, 0x02, 0x20, 0x77, 0x24, 0x01])), (0x05, vec![]), "a class it does not hold");
    assert_eq!(status_of(&c.cip(s, &[0x0E, 0x02, 0x20, 0x01, 0xE0, 0x01])), (0x04, vec![1]), "a segment it cannot read, its word named");
    assert_eq!(status_of(&c.cip(s, &[0x0E, 0x09, 0x20])), (0x13, vec![]), "a path longer than the request");
    // The Identity's status word owns bit 0 while a connection is open.
    let mut o = Originator::connect(&adapter);
    o.open(&adapter, &open_for(4, 8));
    assert_eq!(&single(&mut c, 5)[4..], &[1, 0]);
}

#[test]
fn the_assembly_object_reads_and_writes_the_data_the_plant_side_holds() {
    let adapter = start(&config(&device(44))).unwrap();
    let mut c = Encap::connect(adapter.addr);
    let s = c.register();
    let get = |c: &mut Encap, inst: u8, attr: u8| c.cip(s, &[0x0E, 0x03, 0x20, 0x04, 0x24, inst, 0x30, attr]);
    let set = |c: &mut Encap, inst: u8, data: &[u8]| {
        let mut r = vec![0x10, 0x03, 0x20, 0x04, 0x24, inst, 0x30, 0x03];
        r.extend_from_slice(data);
        c.cip(s, &r)
    };
    // The input assembly as the points' initial values left it: speed 12.5 at byte 4.
    let mut input = vec![0u8; 8];
    input[4..8].copy_from_slice(&12.5f32.to_le_bytes());
    assert_eq!(get(&mut c, 100, 3), [vec![0x8E, 0, 0, 0], input.clone()].concat());
    assert_eq!(get(&mut c, 100, 4), vec![0x8E, 0, 0, 0, 8, 0]);
    adapter.set("running", "true").unwrap();
    adapter.set_value("speed", Value::F32(-3.0)).unwrap();
    input[0] = 0b10;
    input[4..8].copy_from_slice(&(-3.0f32).to_le_bytes());
    assert_eq!(get(&mut c, 100, 3)[4..], input[..], "the plant side's set, read next");
    // The output assembly written whole lands where the plant side reads it.
    assert_eq!(set(&mut c, 150, &[0x01, 0x00, 0x34, 0x12]), vec![0x90, 0, 0, 0]);
    assert_eq!((adapter.get("start").unwrap(), adapter.get("setpoint").unwrap()), (Value::Bool(true), Value::U16(0x1234)));
    assert_eq!(adapter.assembly(150).unwrap(), [0x01, 0x00, 0x34, 0x12]);
    assert_eq!(status_of(&set(&mut c, 150, &[1, 2, 3])), (0x13, vec![]));
    assert_eq!(status_of(&set(&mut c, 150, &[1, 2, 3, 4, 5])), (0x15, vec![]));
    assert_eq!(adapter.assembly(150).unwrap(), [0x01, 0x00, 0x34, 0x12], "neither refused write landed");
    assert_eq!(status_of(&set(&mut c, 100, &[0; 8])), (0x0E, vec![]), "an assembly the adapter produces");
    assert_eq!(status_of(&get(&mut c, 100, 5)), (0x14, vec![]));
    assert_eq!(status_of(&get(&mut c, 99, 3)), (0x05, vec![]));
    // While a connection consumes the output assembly, an explicit write to it is a conflict.
    let mut o = Originator::connect(&adapter);
    o.open(&adapter, &open_for(4, 8));
    assert_eq!(status_of(&set(&mut c, 150, &[0; 4])), (0x0C, vec![]));
    // The plant side by bytes, and its refusals.
    assert_eq!(adapter.set_assembly(100, &[0; 3]).unwrap_err().to_string(), "assembly 100 holds 8 byte(s), not 3");
    assert_eq!(adapter.assembly(7).unwrap_err().to_string(), "no assembly 7");
    assert_eq!(adapter.set_value("speed", Value::U16(1)).unwrap_err().to_string(), "point `speed` is f32, not u16");
    assert_eq!(adapter.set("speed", "fast").unwrap_err().to_string(), "point `speed`: `fast` is not an f32");
    assert_eq!(adapter.get("nothing").unwrap_err().to_string(), "no point `nothing`");
}

#[test]
fn an_unconnected_send_is_answered_when_its_route_ends_here() {
    let adapter = start(&config(&device(45))).unwrap();
    let mut c = Encap::connect(adapter.addr);
    let s = c.register();
    let inner = [0x0E, 0x03, 0x20, 0x01, 0x24, 0x01, 0x30, 0x01];
    let wrapped = |route: &[u8]| {
        let mut r = vec![0x52, 0x02, 0x20, 0x06, 0x24, 0x01, 0x0A, 0x05];
        r.extend_from_slice(&(inner.len() as u16).to_le_bytes());
        r.extend_from_slice(&inner);
        r.push((route.len() / 2) as u8);
        r.push(0);
        r.extend_from_slice(route);
        r
    };
    // An empty route, or the backplane's slot 0: the embedded request's own reply.
    assert_eq!(c.cip(s, &wrapped(&[])), vec![0x8E, 0, 0, 0, 9, 0]);
    assert_eq!(c.cip(s, &wrapped(&[0x01, 0x00])), vec![0x8E, 0, 0, 0, 9, 0]);
    // Any other route goes nowhere: the Unconnected_Send's own failure.
    let r = c.cip(s, &wrapped(&[0x01, 0x02]));
    assert_eq!((r[0], status_of(&r)), (0xD2, (0x01, vec![0x0312])));
}

#[test]
fn a_class_1_connection_produces_at_its_rpi_and_consumes_what_its_originator_runs() {
    let adapter = start(&config(&device(46))).unwrap();
    let mut o = Originator::connect(&adapter);
    let open = open_for(4, 8);
    let (reply, to) = o.open(&adapter, &open);
    assert_eq!(
        (reply.t_to_o_connection_id, reply.connection_serial, reply.originator_vendor, reply.originator_serial, reply.o_to_t_actual_rpi_us, reply.t_to_o_actual_rpi_us),
        (0x8000_1234, 0x1234, 0x00FE, 0x0BADCAFE, 20_000, 20_000)
    );
    assert!(reply.app_reply.is_empty());
    let o_to_t = reply.o_to_t_connection_id;
    // Produced: the input assembly, modeless, to the originator's socket, at its RPI.
    let mut input = vec![0u8; 8];
    input[4..8].copy_from_slice(&12.5f32.to_le_bytes());
    let first = o.next_input(false);
    assert_eq!((first.connection_id, first.run_idle, first.data.clone()), (0x8000_1234, None, input.clone()));
    let started = Instant::now();
    let mut frames = vec![first];
    while started.elapsed() < Duration::from_millis(1000) {
        frames.push(o.next_input(false));
    }
    let n = frames.len();
    assert!((45..=56).contains(&n), "{n} frames in a second at a 20 ms RPI");
    for w in frames.windows(2) {
        assert_eq!((w[1].sequence, w[1].cip_sequence), (w[0].sequence.wrapping_add(1), w[0].cip_sequence.wrapping_add(1)), "each frame counted on");
    }
    // The plant side's set is in the next frames.
    adapter.set_value("speed", Value::F32(99.0)).unwrap();
    let mut seen = false;
    for _ in 0..10 {
        if o.next_input(false).data[4..8] == 99.0f32.to_le_bytes() {
            seen = true;
            break;
        }
    }
    assert!(seen, "the plant side's value produced");
    // Consumed: a run frame's data lands in the output assembly.
    o.send_output(to, o_to_t, 1, 1, true, &[0x01, 0x00, 0x34, 0x12]);
    assert!(wait(Duration::from_secs(5), || adapter.get("setpoint").unwrap() == Value::U16(0x1234)));
    assert_eq!(adapter.get("start").unwrap(), Value::Bool(true));
    // The same sequence count again is no new data; an idle frame keeps the outputs as they are.
    o.send_output(to, o_to_t, 2, 1, true, &[0x00, 0x00, 0x99, 0x99]);
    o.send_output(to, o_to_t, 3, 2, false, &[0x00, 0x00, 0x77, 0x77]);
    assert!(wait(Duration::from_secs(5), || adapter.status().connections[0].consumed == 3));
    assert_eq!(adapter.assembly(150).unwrap(), [0x01, 0x00, 0x34, 0x12]);
    assert_eq!(adapter.status().connections[0].run, Some(false));
    o.send_output(to, o_to_t, 4, 3, true, &[0x00, 0x00, 0x01, 0x00]);
    assert!(wait(Duration::from_secs(5), || adapter.get("setpoint").unwrap() == Value::U16(1)));
    assert_eq!(adapter.status().connections[0].run, Some(true));
    // A frame of the wrong size, or for no connection, is stray and changes nothing.
    o.send_output(to, o_to_t, 5, 4, true, &[0xFF; 5]);
    o.send_output(to, 0xDEAD, 6, 5, true, &[0xFF; 4]);
    assert!(wait(Duration::from_secs(5), || adapter.status().stray_frames == 2));
    assert_eq!(adapter.get("setpoint").unwrap(), Value::U16(1));
    // Forward_Close: closed, produced no more; a second close finds nothing.
    let r = o.forward_close(0x1234);
    let closed = vec![0xCE, 0, 0, 0, 0x34, 0x12, 0xFE, 0x00, 0xFE, 0xCA, 0xAD, 0x0B, 0, 0];
    assert_eq!(r, closed);
    std::thread::sleep(Duration::from_millis(100));
    let mut buf = [0u8; 600];
    o.udp.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
    while o.udp.recv_from(&mut buf).is_ok() {}
    o.udp.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
    assert!(o.udp.recv_from(&mut buf).is_err(), "nothing produced after the close");
    let r = o.forward_close(0x1234);
    assert_eq!(status_of(&r), (0x01, vec![0x0107]));
    let s = adapter.status();
    assert_eq!((s.connections.len(), s.closed, s.timed_out), (0, 1, 0));
}

#[test]
fn a_connection_whose_originator_falls_silent_is_closed_at_its_timeout() {
    let adapter = start(&config(&device(47))).unwrap();
    let mut o = Originator::connect(&adapter);
    // 10 ms RPI, multiplier 0: a timeout of 40 ms once frames have come.
    let open = Open { rpi_us: 10_000, multiplier: 0, ..open_for(4, 8) };
    let (reply, to) = o.open(&adapter, &open);
    for i in 0..10u32 {
        o.send_output(to, reply.o_to_t_connection_id, i, i as u16, true, &[0; 4]);
        std::thread::sleep(Duration::from_millis(10));
    }
    let silent = Instant::now();
    assert!(wait(Duration::from_secs(5), || adapter.status().timed_out == 1));
    let after = silent.elapsed();
    assert!(after >= Duration::from_millis(30), "closed {after:?} after the last frame, before its 40 ms");
    let s = adapter.status();
    assert_eq!((s.connections.len(), s.closed), (0, 1));
    // The output is free again: a new connection opens.
    o.open(&adapter, &Open { serial: 0x2222, ..open_for(4, 8) });
}

#[test]
fn every_forward_open_the_adapter_cannot_honour_is_refused_with_its_status() {
    let text = format!("{}max_connections = 2\nassemblies = [ {{ instance = 100, size = 8 }}, {{ instance = 150, size = 4 }}, {{ instance = 151, size = 3 }}, {{ instance = 152, size = 2 }}, {{ instance = 101, size = 2 }} ]\nconnections = [ {{ config = 151, output = 150, input = 100 }}, {{ config = 151, output = 152, input = 101 }}, {{ config = 151, output = 101, input = 100 }} ]\n",
        device(48).replace("assemblies = [ { instance = 100, size = 8 }, { instance = 150, size = 4 }, { instance = 151, size = 2 } ]\n", "").replace("connections = [ { config = 151, output = 150, input = 100 } ]\n", ""));
    let adapter = start(&config(&text)).unwrap();
    let mut o = Originator::connect(&adapter);
    let base = open_for(4, 8);
    let refuse = |o: &mut Originator, open: &Open| -> Vec<u16> {
        let (r, _) = o.forward_open(open);
        let (general, additional) = status_of(&r);
        assert_eq!((r[0], general), (0xD4, 0x01), "{r:02X?}");
        // The originator's triple and a remaining path size of 0, after the status words.
        let at = 4 + 2 * additional.len();
        let mut triple = open.serial.to_le_bytes().to_vec();
        triple.extend_from_slice(&[0xFE, 0x00, 0xFE, 0xCA, 0xAD, 0x0B, 0, 0]);
        assert_eq!(&r[at..], &triple[..], "{r:02X?}");
        additional
    };
    // Sizes: the protocol's count the 16-bit sequence count (and the run/idle header where the
    // direction has one); a size without them is refused, the size wanted beside the status.
    assert_eq!(refuse(&mut o, &Open { o_to_t_size: 4 + 4, ..base.clone() }), [0x0127, 10]);
    assert_eq!(refuse(&mut o, &Open { t_to_o_size: 8, ..base.clone() }), [0x0128, 10]);
    assert_eq!(refuse(&mut o, &Open { transport: 0x03, ..base.clone() }), [0x0103], "class 3");
    assert_eq!(refuse(&mut o, &Open { transport: 0x11, ..base.clone() }), [0x0103], "change of state");
    assert_eq!(refuse(&mut o, &Open { t_to_o_kind: 1, ..base.clone() }), [0x0108], "a multicast T->O");
    assert_eq!(refuse(&mut o, &Open { o_to_t_kind: 0, ..base.clone() }), [0x0108], "a null O->T");
    assert_eq!(refuse(&mut o, &Open { config: 152, ..base.clone() }), [0x0129]);
    assert_eq!(refuse(&mut o, &Open { output: 152, input: 100, ..base.clone() }), [0x012B], "152 consumed, but with 101 only");
    assert_eq!(refuse(&mut o, &Open { output: 100, ..base.clone() }), [0x012A]);
    assert_eq!(refuse(&mut o, &Open { config_data: Some(vec![1, 2]), ..base.clone() }), [0x0126, 3]);
    assert_eq!(refuse(&mut o, &Open { rpi_us: 500, ..base.clone() }), [0x0111]);
    assert_eq!(refuse(&mut o, &Open { rpi_us: 10_000_001, ..base.clone() }), [0x0111]);
    // The electronic key: vendor, device type, product code, revision (the compatibility bit
    // takes any minor up to the device's).
    let key = |vendor: u16, device: u16, product: u16, major: u8, minor: u8| {
        let mut k = [0x34, 0x04, 0, 0, 0, 0, 0, 0, major, minor];
        k[2..4].copy_from_slice(&vendor.to_le_bytes());
        k[4..6].copy_from_slice(&device.to_le_bytes());
        k[6..8].copy_from_slice(&product.to_le_bytes());
        Some(k)
    };
    assert_eq!(refuse(&mut o, &Open { key: key(8, 7, 3, 2, 5), ..base.clone() }), [0x0114]);
    assert_eq!(refuse(&mut o, &Open { key: key(9, 7, 4, 2, 5), ..base.clone() }), [0x0114]);
    assert_eq!(refuse(&mut o, &Open { key: key(9, 6, 3, 2, 5), ..base.clone() }), [0x0115]);
    assert_eq!(refuse(&mut o, &Open { key: key(9, 7, 3, 3, 5), ..base.clone() }), [0x0116]);
    assert_eq!(refuse(&mut o, &Open { key: key(9, 7, 3, 2, 4), ..base.clone() }), [0x0116], "exact: the minor must match");
    assert_eq!(refuse(&mut o, &Open { key: key(9, 7, 3, 0x82, 6), ..base.clone() }), [0x0116], "compatible: a minor past the device's");
    let accepted = |o: &mut Originator, open: &Open| {
        let (r, _) = o.forward_open(open);
        assert_eq!(&r[..4], &[0xD4, 0, 0, 0], "{r:02X?}");
        o.forward_close(open.serial);
    };
    accepted(&mut o, &Open { key: key(9, 7, 3, 0x82, 4), ..base.clone() });
    accepted(&mut o, &Open { key: key(0, 0, 0, 0, 0), ..base.clone() });
    accepted(&mut o, &Open { config_data: Some(vec![7, 8, 9]), ..base.clone() });
    assert_eq!(adapter.assembly(151).unwrap(), [7, 8, 9], "the configuration data written to its assembly");
    // A triple already open; an output already owned; the connection limit.
    o.open(&adapter, &base);
    assert_eq!(refuse(&mut o, &base), [0x0100]);
    assert_eq!(refuse(&mut o, &Open { serial: 0x1235, ..base.clone() }), [0x0106]);
    let mut p = Originator::connect(&adapter);
    p.open(&adapter, &Open { output: 152, input: 101, o_to_t_size: 2 + 6, t_to_o_size: 2 + 2, serial: 0x0001, ..base.clone() });
    let (r, _) = p.forward_open(&Open { output: 101, input: 100, o_to_t_size: 2 + 6, serial: 0x0002, ..base.clone() });
    assert_eq!(status_of(&r), (0x01, vec![0x0113]));
    let s = adapter.status();
    assert_eq!(s.refused_opens.get(&0x0116), Some(&3));
    assert_eq!(s.connections.len(), 2);
}

#[test]
fn the_crate_s_own_scanner_leaves_the_sequence_count_out_and_is_refused_for_it() {
    // `ethernetip-connections` 0.1.1's scanner states a connection's size without the 16-bit
    // sequence count; an adapter that holds to the protocol refuses it, naming the size it wants.
    let adapter = start(&config(&device(49))).unwrap();
    let addr = adapter.addr;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let e = rt
        .block_on(ethernetip_connections::open_scanner_connection(ethernetip_connections::ScannerConfig::new(
            addr,
            ethernetip_connections::AssemblyRegistry::new(),
            151,
            150,
            100,
            4,
            8,
        )))
        .err()
        .expect("refused");
    assert!(matches!(e, ethernetip_core::EipError::Cip { status: 0x01, .. }), "{e:?}");
    assert_eq!(adapter.status().refused_opens.get(&0x0127), Some(&1));
}

#[test]
fn the_knobs_stall_production_silence_the_sessions_and_restart_the_device() {
    let adapter = start(&config(&device(50))).unwrap();
    let mut o = Originator::connect(&adapter);
    let (reply, to) = o.open(&adapter, &open_for(4, 8));
    o.next_input(false);
    // Production stalled: the originator's inputs fall silent; on again, they come.
    adapter.set_producing(false);
    std::thread::sleep(Duration::from_millis(60));
    let mut buf = [0u8; 600];
    o.udp.set_read_timeout(Some(Duration::from_millis(20))).unwrap();
    while o.udp.recv_from(&mut buf).is_ok() {}
    o.udp.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
    assert!(o.udp.recv_from(&mut buf).is_err(), "no frame while production is stalled");
    adapter.set_producing(true);
    o.udp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    o.next_input(false);
    // Silent: no encapsulation request answered, the connection kept; Class 1 runs on.
    adapter.set_silent(true);
    o.encap.send(&Encap::frame(0x63, 0, [0; 8], &[]));
    o.encap.stream.set_read_timeout(Some(Duration::from_millis(400))).unwrap();
    let mut b = [0u8; 1];
    assert!(o.encap.stream.read(&mut b).is_err(), "no answer while silent");
    o.next_input(false);
    adapter.set_silent(false);
    o.encap.stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let (h, _) = o.encap.ask(0x63, 0, &[]);
    assert_eq!(h.command, 0x63);
    // Restart: every session's connection and every Class 1 connection closed; it listens on.
    o.send_output(to, reply.o_to_t_connection_id, 1, 1, true, &[0; 4]);
    adapter.restart();
    assert_eq!(o.encap.stream.read(&mut b).unwrap_or(0), 0, "the session's connection closed");
    let s = adapter.status();
    assert_eq!((s.connections.len(), s.tcp_connections), (0, 0));
    let mut again = Originator::connect(&adapter);
    again.open(&adapter, &Open { serial: 0x3333, ..open_for(4, 8) });
}

#[test]
fn stopping_closes_everything_and_frees_every_port() {
    let adapter = start(&config(&device(51))).unwrap();
    let (addr, io) = (adapter.addr, adapter.io_addr);
    let mut o = Originator::connect(&adapter);
    o.open(&adapter, &open_for(4, 8));
    adapter.stop();
    let mut b = [0u8; 1];
    assert_eq!(o.encap.stream.read(&mut b).unwrap_or(0), 0);
    assert!(std::net::TcpListener::bind(addr).is_ok(), "the encapsulation port, TCP");
    assert!(UdpSocket::bind(addr).is_ok(), "the encapsulation port, UDP");
    assert!(UdpSocket::bind(io).is_ok(), "the Class 1 port");
    // A port taken is refused by name, before anything else binds.
    let taken = std::net::TcpListener::bind("127.0.0.52:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let e = start(&config(&format!("bind = \"127.0.0.52\"\nport = {port}\nio_port = 0\n"))).unwrap_err();
    assert!(e.to_string().starts_with(&format!("bind 127.0.0.52:{port} (tcp): ")), "{e}");
}

/// A capture of `segments` (true: from the originator) as one TCP conversation to port 44818, and
/// `datagrams` as UDP to and from port 2222, for the dissectors.
fn capture(segments: &[(bool, Vec<u8>)], datagrams: &[(bool, Vec<u8>)]) -> Vec<Vec<u8>> {
    fn sum(b: &[u8]) -> u16 {
        let mut s: u32 = b.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)]) as u32).sum();
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
    /// A TCP segment between `ends` (address and port each side), its checksum over the pseudo
    /// header.
    fn tcp(ends: (([u8; 4], u16), ([u8; 4], u16)), seq: u32, ack: u32, flags: u8, data: &[u8]) -> Vec<u8> {
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
    let mut out = vec![tcp((o, t), sa - 1, 0, 0x02, &[]), tcp((t, o), sb - 1, sa, 0x12, &[]), tcp((o, t), sa, sb, 0x10, &[])];
    for (from_originator, data) in segments {
        if *from_originator {
            out.push(tcp((o, t), sa, sb, 0x18, data));
            sa += data.len() as u32;
        } else {
            out.push(tcp((t, o), sb, sa, 0x18, data));
            sb += data.len() as u32;
        }
    }
    for (from_originator, data) in datagrams {
        let (src, dst) = if *from_originator { (a, b) } else { (b, a) };
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
        if Command::new(c).arg("--version").output().map(|o| o.status.success()).unwrap_or(false) {
            return c.clone();
        }
    }
    panic!("the dissectors are this test's second reading of the standard, and no tshark runs here: tried {candidates:?}; set VEREDAS_TSHARK or extract tshark into ~/.local/opt/wireshark");
}

/// Every packet's layers as the dissectors read them (`-T ek`).
fn dissect(frames: &[Vec<u8>]) -> Vec<Json> {
    let dir = std::env::temp_dir().join(format!("veredas-enip-dissect-{}", std::process::id()));
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
    let out = Command::new(tshark()).arg("-n").arg("-r").arg(&file).args(["-T", "ek"]).output().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert!(out.status.success(), "tshark: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().lines().filter(|l| !l.starts_with("{\"index\"")).map(|l| serde_json::from_str::<Json>(l).unwrap()["layers"].clone()).collect()
}

fn values(v: &Json, key: &str, out: &mut Vec<String>) {
    match v {
        Json::Object(map) => {
            for (k, x) in map {
                if k == key {
                    match x {
                        Json::Array(items) => out.extend(items.iter().map(|i| i.as_str().map(String::from).unwrap_or_else(|| i.to_string()))),
                        other => out.push(other.as_str().map(String::from).unwrap_or_else(|| other.to_string())),
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

#[test]
fn the_dissectors_read_a_whole_conversation_as_the_protocol_has_it() {
    let adapter = start(&config(&device(53))).unwrap();
    let mut o = Originator::connect(&adapter);
    o.encap.ask(0x63, 0, &[]);
    o.encap.cip(o.session, &[0x01, 0x02, 0x20, 0x01, 0x24, 0x01]);
    o.encap.cip(o.session, &[0x0E, 0x03, 0x20, 0x04, 0x24, 100, 0x30, 0x03]);
    let (reply, to) = o.open(&adapter, &open_for(4, 8));
    let mut datagrams = Vec::new();
    for i in 0..3u32 {
        let frame = ethernetip_connections::encode_frame(&ethernetip_connections::Frame {
            connection_id: reply.o_to_t_connection_id,
            sequence: i,
            cip_sequence: i as u16,
            run_idle: Some(true),
            data: vec![1, 0, 0x34, 0x12],
        });
        o.udp.send_to(&frame, to).unwrap();
        datagrams.push((true, frame));
        let mut buf = [0u8; 600];
        let (n, _) = o.udp.recv_from(&mut buf).unwrap();
        datagrams.push((false, buf[..n].to_vec()));
    }
    o.forward_close(0x1234);
    // The register's request is in the log too: the whole TCP conversation, then the datagrams.
    let packets = dissect(&capture(&o.encap.log, &datagrams));
    let enip: Vec<&Json> = packets.iter().filter(|p| !p["enip"].is_null()).collect();
    assert_eq!(enip.len(), o.encap.log.len() + datagrams.len(), "every frame read as EtherNet/IP");
    for p in &packets {
        assert_eq!(complaints(p), Vec::<String>::new());
    }
    let all = |key: &str| packets.iter().flat_map(|p| field(p, key)).collect::<Vec<_>>();
    assert_eq!(all("enip_enip_lir_vendor"), ["9"]);
    assert_eq!(all("enip_enip_lir_devtype"), ["7"]);
    assert_eq!(all("enip_enip_lir_prodcode"), ["3"]);
    assert_eq!(all("enip_enip_lir_serial"), ["305419896"], "0x12345678, as the dissector's EK output writes numbers");
    assert_eq!(all("enip_enip_lir_name"), ["bench io"]);
    // The Forward_Open as the dissector reads it: the sizes, the RPIs, the connection points.
    assert_eq!(all("cipcm_cip_cm_fwo_consize"), ["10", "10"]);
    assert_eq!(all("cipcm_cip_cm_otrpi"), ["20000"]);
    assert_eq!(all("cipcm_cip_cm_ot_connid").last(), Some(&reply.o_to_t_connection_id.to_string()));
    let statuses = all("cip_cip_genstat");
    assert!(!statuses.is_empty() && statuses.iter().all(|g| g == "0"), "{statuses:?}");
}

#[test]
fn the_documented_adapter_runs_from_its_bench_file() {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/bench-schema.md")).unwrap().replace("\r\n", "\n");
    let mut rest = text.as_str();
    let mut blocks = Vec::new();
    while let Some(start) = rest.find("```toml\n") {
        let body = &rest[start + 8..];
        let end = body.find("```").unwrap();
        blocks.push(body[..end].to_string());
        rest = &body[end + 3..];
    }
    let block = blocks.into_iter().find(|b| b.contains("[[ethernetip]]")).expect("the reference's EtherNet/IP example");
    let text = block.replace("port = 44818", "port = 0").replace("io_port = 2222", "io_port = 0").replace("bind = \"127.0.0.1\"", "bind = \"127.0.0.54\"");
    let spec = veredas_maquinar::bench::load(&text, Path::new(".")).unwrap();
    let plant = veredas_maquinar::bench::Bench::start(spec).unwrap();
    let adapter = plant.ethernetip("remote-io").unwrap();
    let mut o = Originator::connect(adapter);
    let (reply, to) = o.open(adapter, &Open { config: 151, output: 150, input: 100, o_to_t_size: 2 + 4 + 2, t_to_o_size: 2 + 4, ..open_for(2, 4) });
    o.send_output(to, reply.o_to_t_connection_id, 1, 1, true, &[0x01, 0x00]);
    assert!(wait(Duration::from_secs(5), || adapter.get("start").unwrap() == Value::Bool(true)));
    adapter.set("ready", "true").unwrap();
    assert!(wait(Duration::from_secs(5), || o.next_input(false).data[0] & 1 == 1));
    let lines = plant.lines();
    assert!(lines[0].starts_with(&format!("ethernetip remote-io: {} (class 1 on {})", adapter.addr, adapter.io_addr)), "{lines:?}");
    plant.stop();
}

// ---- C-111, refute round 1: the pins (adapted from the critic's probes) ----

/// A request wrapped in an Unconnected_Send with `route`.
fn wrap_us(inner: &[u8], route: &[u8]) -> Vec<u8> {
    let mut r = vec![0x52, 0x02, 0x20, 0x06, 0x24, 0x01, 0x0A, 0x05];
    r.extend_from_slice(&(inner.len() as u16).to_le_bytes());
    r.extend_from_slice(inner);
    if inner.len() % 2 == 1 {
        r.push(0);
    }
    r.push((route.len() / 2) as u8);
    r.push(0);
    r.extend_from_slice(route);
    r
}

/// A Large_Forward_Open request (32-bit network parameters) for 151, 150, 100.
fn large_fo(serial: u16, o_rpi: u32, t_rpi: u32, o_size: u16, t_size: u16) -> Vec<u8> {
    let mut b = vec![0x5B, 0x02, 0x20, 0x06, 0x24, 0x01, 0x0A, 0x05];
    b.extend_from_slice(&0u32.to_le_bytes());
    b.extend_from_slice(&(0x8000_0000u32 | serial as u32).to_le_bytes());
    b.extend_from_slice(&serial.to_le_bytes());
    b.extend_from_slice(&0x00FEu16.to_le_bytes());
    b.extend_from_slice(&0x0BADCAFEu32.to_le_bytes());
    b.extend_from_slice(&[2, 0, 0, 0]);
    b.extend_from_slice(&o_rpi.to_le_bytes());
    b.extend_from_slice(&((2u32 << 29) | o_size as u32).to_le_bytes());
    b.extend_from_slice(&t_rpi.to_le_bytes());
    b.extend_from_slice(&((2u32 << 29) | t_size as u32).to_le_bytes());
    b.push(0x01);
    let path = [0x20, 0x04, 0x24, 151, 0x2C, 150, 0x2C, 100];
    b.push((path.len() / 2) as u8);
    b.extend_from_slice(&path);
    b
}

fn count_frames(o: &Originator, within: Duration) -> usize {
    let started = Instant::now();
    let mut n = 0;
    while started.elapsed() < within {
        o.next_input(false);
        n += 1;
    }
    n
}

#[test]
fn an_unconnected_send_nested_in_another_is_refused_and_the_adapter_lives_on() {
    let adapter = start(&config(&device(60))).unwrap();
    let mut c = Encap::connect(adapter.addr);
    let s = c.register();
    let inner = [0x0E, 0x03, 0x20, 0x01, 0x24, 0x01, 0x30, 0x01];
    // Two levels: the outer one's answer is the inner one's refusal, 0x08 for an Unconnected_Send.
    assert_eq!(c.cip(s, &wrap_us(&wrap_us(&inner, &[]), &[])), vec![0xD2, 0, 0x08, 0]);
    // A thousand levels, which overflowed the stack before: refused the same, and the adapter
    // answers the next request.
    let mut msg = inner.to_vec();
    for _ in 0..1000 {
        msg = wrap_us(&msg, &[]);
    }
    assert_eq!(c.cip(s, &msg), vec![0xD2, 0, 0x08, 0]);
    assert_eq!(c.cip(s, &wrap_us(&inner, &[])), vec![0x8E, 0, 0, 0, 9, 0]);
}

#[test]
fn a_large_forward_open_with_unequal_rpis_is_taken_or_refused_as_a_forward_open_is() {
    let adapter = start(&config(&device(63))).unwrap();
    let mut o = Originator::connect(&adapter);
    let SocketAddr::V4(me) = o.udp.local_addr().unwrap() else { panic!() };
    let sockaddr = ethernetip_core::cpf::encode_sockaddr_in_v4(SocketAddr::V4(me)).unwrap();
    // O→T 50 ms, T→O 10 ms: the reply states each, and the inputs come every 10 ms.
    let session = o.session;
    let env = o.encap.rr(session, &large_fo(0x4444, 50_000, 10_000, 4 + 6, 8 + 2), vec![ethernetip_core::cpf::Item::new(0x8001, sockaddr)]);
    let reply = env.find(0x00B2).unwrap().data.clone();
    assert_eq!(&reply[..4], &[0xDB, 0, 0, 0]);
    let api = |at: usize| u32::from_le_bytes(reply[4 + at..4 + at + 4].try_into().unwrap());
    assert_eq!((api(16), api(20)), (50_000, 10_000), "O→T then T→O");
    o.next_input(false);
    let n = count_frames(&o, Duration::from_secs(1));
    assert!((80..=110).contains(&n), "{n} frames in a second at a 10 ms T→O RPI");
    // Refused for its size as a Forward_Open is, the size wanted beside it.
    let env = o.encap.rr(session, &large_fo(0x4445, 50_000, 10_000, 4, 8 + 2), vec![]);
    assert_eq!(status_of(&env.find(0x00B2).unwrap().data), (0x01, vec![0x0127, 10]));
    // A T→O RPI alone outside the range: refused 0x0111.
    let mut req = o.request(&open_for(4, 8));
    req.t_to_o_rpi_us = 500;
    let mut mr = vec![0x54, 0x02, 0x20, 0x06, 0x24, 0x01];
    mr.extend(req.encode());
    assert_eq!(status_of(&o.encap.cip(o.session, &mr)), (0x01, vec![0x0111]));
}

#[test]
fn the_timeout_counts_the_consumed_rpi_and_the_multiplier_and_the_first_frame_waits_ten_seconds() {
    let adapter = start(&config(&device(77))).unwrap();
    let mut o = Originator::connect(&adapter);
    // O→T 10 ms, T→O 30 ms, multiplier 1: 10 ms × 4 << 1 = 80 ms from the last frame.
    let mut req = o.request(&Open { rpi_us: 10_000, multiplier: 1, ..open_for(4, 8) });
    req.t_to_o_rpi_us = 30_000;
    let mut mr = vec![0x54, 0x02, 0x20, 0x06, 0x24, 0x01];
    mr.extend(req.encode());
    let SocketAddr::V4(me) = o.udp.local_addr().unwrap() else { panic!() };
    let sockaddr = ethernetip_core::cpf::encode_sockaddr_in_v4(SocketAddr::V4(me)).unwrap();
    let env = o.encap.rr(o.session, &mr, vec![ethernetip_core::cpf::Item::new(0x8001, sockaddr)]);
    let reply = env.find(0x00B2).unwrap().data.clone();
    assert_eq!(reply[2], 0);
    let to = env.find(0x8000).map(|i| SocketAddr::V4(ethernetip_core::cpf::decode_sockaddr_in_v4(&i.data).unwrap())).unwrap();
    let id = u32::from_le_bytes(reply[4..8].try_into().unwrap());
    let mut last = Instant::now();
    for i in 0..10u32 {
        o.send_output(to, id, i, i as u16, true, &[0; 4]);
        last = Instant::now();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(wait(Duration::from_secs(5), || adapter.status().timed_out == 1));
    // From the last frame sent, which the adapter took no sooner: 80 ms is a floor.
    let after = last.elapsed();
    assert!(after >= Duration::from_millis(80) && after < Duration::from_millis(200), "{after:?}");
    // No frame at all: still open at 9 s, closed by 10 s and a little.
    let mut p = Originator::connect(&adapter);
    p.open(&adapter, &Open { rpi_us: 10_000, multiplier: 0, serial: 0x0F0F, ..open_for(4, 8) });
    std::thread::sleep(Duration::from_millis(9_000));
    assert_eq!(adapter.status().connections.len(), 1, "closed before its 10 s");
    assert!(wait(Duration::from_secs(3), || adapter.status().timed_out == 2));
}

#[test]
fn the_configured_headers_carry_the_frames_both_ways() {
    let text = device(66).replace("connections = [ { config = 151, output = 150, input = 100 } ]", "connections = [ { config = 151, output = 150, input = 100, output_header = \"modeless\", input_header = \"run-idle\" } ]");
    let adapter = start(&config(&text)).unwrap();
    let mut o = Originator::connect(&adapter);
    // Sizes with the headers swapped: O→T the data and the count, T→O those and run/idle.
    let (reply, to) = o.open(&adapter, &Open { o_to_t_size: 4 + 2, t_to_o_size: 8 + 2 + 4, ..open_for(4, 8) });
    let f = o.next_input(true);
    assert_eq!((f.run_idle, f.data.len()), (Some(true), 8));
    let frame = ethernetip_connections::encode_frame(&ethernetip_connections::Frame { connection_id: reply.o_to_t_connection_id, sequence: 1, cip_sequence: 1, run_idle: None, data: vec![1, 0, 0x66, 0x66] });
    o.udp.send_to(&frame, to).unwrap();
    assert!(wait(Duration::from_secs(2), || adapter.get("setpoint").unwrap() == Value::U16(0x6666)));
}

#[test]
fn a_frame_late_from_its_originator_or_from_another_host_is_kept_out() {
    let adapter = start(&config(&device(64))).unwrap();
    let mut o = Originator::connect(&adapter);
    let (reply, to) = o.open(&adapter, &open_for(4, 8));
    let id = reply.o_to_t_connection_id;
    o.send_output(to, id, 10, 10, true, &[1, 0, 0x11, 0x11]);
    assert!(wait(Duration::from_secs(5), || adapter.get("setpoint").unwrap() == Value::U16(0x1111)));
    // An older sequence number, late: dropped, the newer data kept.
    o.send_output(to, id, 9, 9, true, &[1, 0, 0x22, 0x22]);
    o.send_output(to, id, 1, 11, true, &[1, 0, 0x33, 0x33]);
    assert!(wait(Duration::from_secs(5), || adapter.status().stale_frames == 2));
    assert_eq!(adapter.get("setpoint").unwrap(), Value::U16(0x1111));
    // Forward in steps under 2^31 to the top and across the wrap: 0 is newer than 0xFFFFFFFF, and
    // 0xFFFFFFFE after it is late again.
    for (sequence, count) in [(0x7000_0000, 12), (0xE000_0000, 13), (0xFFFF_FFFF, 14)] {
        o.send_output(to, id, sequence, count, true, &[1, 0, 0x44, count as u8]);
        assert!(wait(Duration::from_secs(5), || adapter.get("setpoint").unwrap() == Value::U16(u16::from_le_bytes([0x44, count as u8]))), "{sequence:#X}");
    }
    o.send_output(to, id, 0, 15, true, &[1, 0, 0x55, 0x55]);
    assert!(wait(Duration::from_secs(5), || adapter.get("setpoint").unwrap() == Value::U16(0x5555)));
    o.send_output(to, id, 0xFFFF_FFFE, 16, true, &[1, 0, 0x66, 0x66]);
    assert!(wait(Duration::from_secs(5), || adapter.status().stale_frames == 3));
    assert_eq!(adapter.get("setpoint").unwrap(), Value::U16(0x5555));
    // A valid frame from another host than the originator's: stray.
    let other = UdpSocket::bind("127.0.0.2:0").unwrap();
    let frame = ethernetip_connections::encode_frame(&ethernetip_connections::Frame { connection_id: id, sequence: 1, cip_sequence: 99, run_idle: Some(true), data: vec![1, 0, 0x66, 0x66] });
    other.send_to(&frame, to).unwrap();
    assert!(wait(Duration::from_secs(5), || adapter.status().stray_frames == 1));
    assert_eq!(adapter.get("setpoint").unwrap(), Value::U16(0x5555));
}

#[test]
fn produced_frames_go_to_the_originator_s_host_at_its_port_or_the_standard_one() {
    // The item names another host: the frames go to the session's host at the item's port.
    let adapter = start(&config(&device(67))).unwrap();
    let mut o = Originator::connect(&adapter);
    let third = UdpSocket::bind("127.0.0.9:0").unwrap();
    third.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
    let mine = UdpSocket::bind(("127.0.0.1", third.local_addr().unwrap().port())).unwrap();
    mine.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut mr = vec![0x54, 0x02, 0x20, 0x06, 0x24, 0x01];
    mr.extend(o.request(&open_for(4, 8)).encode());
    let sockaddr = ethernetip_core::cpf::encode_sockaddr_in_v4(third.local_addr().unwrap()).unwrap();
    let env = o.encap.rr(o.session, &mr, vec![ethernetip_core::cpf::Item::new(0x8001, sockaddr)]);
    assert_eq!(env.find(0x00B2).unwrap().data[2], 0);
    let mut buf = [0u8; 600];
    assert!(mine.recv_from(&mut buf).is_ok(), "the session's host, at the item's port");
    assert!(third.recv_from(&mut buf).is_err(), "never another host");
    // No item: the session's host at 2222, on an adapter whose own I/O port is 2222 (no O→T item
    // in its reply then).
    let Ok(standard) = UdpSocket::bind("127.0.0.1:2222") else { panic!("127.0.0.1:2222 is taken on this machine") };
    standard.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let adapter = start(&config(&device(74).replace("io_port = 0", "io_port = 2222"))).unwrap();
    let mut o = Originator::connect(&adapter);
    let mut mr = vec![0x54, 0x02, 0x20, 0x06, 0x24, 0x01];
    mr.extend(o.request(&open_for(4, 8)).encode());
    let env = o.encap.rr(o.session, &mr, vec![]);
    assert_eq!(env.find(0x00B2).unwrap().data[2], 0);
    assert!(env.find(0x8000).is_none(), "no O→T item on the standard port");
    assert!(standard.recv_from(&mut buf).is_ok(), "the originator's host at 2222");
}

#[test]
fn a_connection_s_first_frame_comes_at_once_whatever_the_others_wait_for() {
    let text = device(68).replace(
        "connections = [ { config = 151, output = 150, input = 100 } ]",
        "connections = [ { config = 151, output = 150, input = 100 }, { config = 151, output = 151, input = 100 } ]",
    );
    let adapter = start(&config(&text)).unwrap();
    let mut a = Originator::connect(&adapter);
    a.open(&adapter, &Open { rpi_us: 5_000_000, multiplier: 0, ..open_for(4, 8) });
    a.next_input(false);
    std::thread::sleep(Duration::from_millis(200));
    let mut b = Originator::connect(&adapter);
    let opened = Instant::now();
    b.open(&adapter, &Open { output: 151, o_to_t_size: 2 + 2 + 4, serial: 0x0B0B, ..open_for(2, 8) });
    b.next_input(false);
    assert!(opened.elapsed() < Duration::from_millis(200), "{:?}", opened.elapsed());
}

#[test]
fn a_forward_open_inside_an_unconnected_send_keeps_its_items() {
    let adapter = start(&config(&device(75))).unwrap();
    let mut o = Originator::connect(&adapter);
    let mut fo = vec![0x54, 0x02, 0x20, 0x06, 0x24, 0x01];
    fo.extend(o.request(&open_for(4, 8)).encode());
    let SocketAddr::V4(me) = o.udp.local_addr().unwrap() else { panic!() };
    let sockaddr = ethernetip_core::cpf::encode_sockaddr_in_v4(SocketAddr::V4(me)).unwrap();
    let env = o.encap.rr(o.session, &wrap_us(&fo, &[]), vec![ethernetip_core::cpf::Item::new(0x8001, sockaddr)]);
    assert_eq!(&env.find(0x00B2).unwrap().data[..4], &[0xD4, 0, 0, 0]);
    assert!(env.find(0x8000).is_some(), "the O→T item kept");
    o.next_input(false);
}

#[test]
fn a_path_the_adapter_cannot_read_is_refused_0x0315() {
    let adapter = start(&config(&device(76))).unwrap();
    let mut o = Originator::connect(&adapter);
    for path in [vec![0x20, 0x04, 0x24, 151, 0xE0, 0x00, 0x2C, 150, 0x2C, 100], vec![0x20, 0x04, 0x24, 151, 0x2C, 150]] {
        let mut req = o.request(&open_for(4, 8));
        req.connection_path = path.clone();
        let mut mr = vec![0x54, 0x02, 0x20, 0x06, 0x24, 0x01];
        mr.extend(req.encode());
        assert_eq!(status_of(&o.encap.cip(o.session, &mr)), (0x01, vec![0x0315]), "{path:02X?}");
    }
}

#[test]
fn list_identity_reaches_a_broadcast_and_names_the_address_it_answers_from() {
    // Bound to the any-address, a broadcast reaches it; its item names the address it answers
    // from, not 0.0.0.0.
    let adapter = start(&config("bind = \"0.0.0.0\"\nport = 0\nio_port = 0\n")).unwrap();
    let port = adapter.addr.port();
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    udp.set_broadcast(true).unwrap();
    udp.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    udp.send_to(&Encap::frame(0x63, 0, *b"bcast...", &[]), ("127.255.255.255", port)).unwrap();
    let mut buf = [0u8; 600];
    let (n, _) = udp.recv_from(&mut buf).unwrap();
    // After the item count, type and length, the protocol version 1, then the socket address.
    assert!(n > 24 + 8 + 16);
    assert_eq!(&buf[24 + 6..24 + 8 + 8], &[1, 0, 0, 2, (port >> 8) as u8, port as u8, 127, 0, 0, 1]);
    // Silent: the UDP ListIdentity is not answered either.
    adapter.set_silent(true);
    udp.set_read_timeout(Some(Duration::from_millis(300))).unwrap();
    udp.send_to(&Encap::frame(0x63, 0, *b"udpquery", &[]), ("127.0.0.1", port)).unwrap();
    assert!(udp.recv_from(&mut buf).is_err());
}

#[test]
fn the_remaining_refusals_and_the_plant_side_by_bytes() {
    let base = |rest: &str| format!("assemblies = [ {{ instance = 100, size = 8 }}, {{ instance = 150, size = 4 }}, {{ instance = 151, size = 0 }} ]\n{rest}");
    assert_eq!(refused(&base("connections = [ { config = 151, output = 150, input = 100 }, { config = 151, output = 150, input = 100 } ]")), "ethernetip.connections[1]: the same three assemblies twice");
    assert_eq!(refused("assemblies = [ { instance = 5, size = 65001 } ]"), "ethernetip.assemblies[0]: 65001 bytes, over 65000");
    assert_eq!(refused(&base("points = [ { name = \"a\", assembly = 100, byte = 0 }, { name = \"a\", assembly = 100, byte = 1 } ]")), "ethernetip.points[1]: two points are named `a`");
    assert_eq!(refused(&base("points = [ { name = \" a\", assembly = 100, byte = 0 } ]")), "ethernetip.points[0]: a point's name is not empty and has no surrounding whitespace");
    assert_eq!(refused(&base("points = [ { name = \"a\", assembly = 7, byte = 0 } ]")), "ethernetip.points[0] (`a`): no assembly 7");
    // A Class 1 port taken: named.
    let taken = UdpSocket::bind("127.0.0.72:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let e = start(&config(&format!("bind = \"127.0.0.72\"\nport = 0\nio_port = {port}\n"))).unwrap_err();
    assert!(e.to_string().starts_with(&format!("bind 127.0.0.72:{port} (udp, class 1): ")), "{e}");
    // The plant side by bytes, read back by point; the Identity class's revision 1.
    let adapter = start(&config(&device(70))).unwrap();
    adapter.set_assembly(150, &[1, 0, 0x78, 0x56]).unwrap();
    assert_eq!((adapter.get("start").unwrap(), adapter.get("setpoint").unwrap()), (Value::Bool(true), Value::U16(0x5678)));
    let mut c = Encap::connect(adapter.addr);
    let s = c.register();
    assert_eq!(c.cip(s, &[0x0E, 0x03, 0x20, 0x01, 0x24, 0x00, 0x30, 0x01]), vec![0x8E, 0, 0, 0, 1, 0]);
}

/// C-111's second refute round: the critic's probes, kept as pins (the multiplier 4 << m at 2, a
/// Large_Forward_Open past 511 bytes, the O→T RPI alone, an equal sequence, the compatibility
/// bit's major, the first frame at a long RPI, stale frames not feeding the watchdog, an IPv6 bind
/// refused, a request path's port, key or data segment refused, nested misspellings, refusals read
/// by the dissectors).
mod round_2 {
    use super::*;

    fn dissect_log(log: &[(bool, Vec<u8>)]) -> Vec<String> {
        let packets = dissect(&capture(log, &[]));
        let mut out = Vec::new();
        for p in &packets {
            out.extend(complaints(p));
        }
        out
    }

    #[test]
    fn the_refusals_read_by_the_dissectors() {
        let adapter = start(&config(&device(120))).unwrap();
        let mut report = Vec::new();
        // 1. An Unconnected_Send to another route: 0x01/0x0312.
        {
            let mut c = Encap::connect(adapter.addr);
            let s = c.register();
            let r = c.cip(s, &wrap_us(&[0x0E, 0x03, 0x20, 0x01, 0x24, 0x01, 0x30, 0x01], &[0x01, 0x02]));
            report.push(format!("unconnected_send 0x0312 reply {r:02X?}: {:?}", dissect_log(&c.log)));
        }
        // 2. A Forward_Open refused 0x0127 with its size.
        {
            let mut o = Originator::connect(&adapter);
            let (r, _) = o.forward_open(&Open { o_to_t_size: 8, ..open_for(4, 8) });
            report.push(format!("forward_open 0x0127 reply {r:02X?}: {:?}", dissect_log(&o.encap.log)));
        }
        // 3. A path segment error 0x04 with its word.
        {
            let mut c = Encap::connect(adapter.addr);
            let s = c.register();
            let r = c.cip(s, &[0x0E, 0x02, 0x20, 0x01, 0xE0, 0x01]);
            report.push(format!("path 0x04 reply {r:02X?}: {:?}", dissect_log(&c.log)));
        }
        // 4. A Large_Forward_Open taken, then a Forward_Close of nothing (0x0107).
        {
            let mut o = Originator::connect(&adapter);
            let SocketAddr::V4(me) = o.udp.local_addr().unwrap() else { panic!() };
            let sockaddr = ethernetip_core::cpf::encode_sockaddr_in_v4(SocketAddr::V4(me)).unwrap();
            let env = o.encap.rr(o.session, &large_fo(0x4444, 20_000, 20_000, 4 + 6, 8 + 2), vec![ethernetip_core::cpf::Item::new(0x8001, sockaddr)]);
            let r = env.find(0x00B2).unwrap().data.clone();
            let fc = o.forward_close(0x7777);
            report.push(format!("large_forward_open reply {r:02X?} then forward_close {fc:02X?}: {:?}", dissect_log(&o.encap.log)));
            o.forward_close(0x4444);
        }
        // 5. ListServices and ListInterfaces, a RegisterSession refused for its version.
        {
            let mut c = Encap::connect(adapter.addr);
            c.ask(0x04, 0, &[]);
            c.ask(0x64, 0, &[]);
            c.ask(0x65, 0, &[2, 0, 0, 0]);
            report.push(format!("list services/interfaces, register 0x69: {:?}", dissect_log(&c.log)));
        }
        // 6. A Forward_Open whose path size is short of its bytes: 0x13 with no triple.
        {
            let mut o = Originator::connect(&adapter);
            let mut mr = vec![0x54, 0x02, 0x20, 0x06, 0x24, 0x01];
            mr.extend(o.request(&open_for(4, 8)).encode());
            mr.extend_from_slice(&[0, 0]);
            let r = o.encap.cip(o.session, &mr);
            report.push(format!("forward_open with 2 trailing bytes, reply {r:02X?}: {:?}", dissect_log(&o.encap.log)));
        }
        for l in &report {
            eprintln!("PROBE {l}");
        }
        let bad: Vec<_> = report.iter().filter(|l| !l.ends_with(": []")).collect();
        assert!(bad.is_empty(), "{bad:#?}");
    }

    #[test]
    fn a_multiplier_of_two_is_sixteen_rpis() {
        let adapter = start(&config(&device(121))).unwrap();
        let mut o = Originator::connect(&adapter);
        // O→T 10 ms, multiplier 2: 10 ms × (4 << 2) = 160 ms; linear (4 × 3) would be 120 ms.
        let (reply, to) = o.open(&adapter, &Open { rpi_us: 10_000, multiplier: 2, ..open_for(4, 8) });
        let mut last = Instant::now();
        for i in 0..10u32 {
            o.send_output(to, reply.o_to_t_connection_id, i, i as u16, true, &[0; 4]);
            last = Instant::now();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(wait(Duration::from_secs(5), || adapter.status().timed_out == 1));
        // From the last frame sent: the adapter took it no sooner, so 160 ms is a floor; linear
        // (4 × 3) would be 120.
        let after = last.elapsed();
        assert!(after >= Duration::from_millis(160) && after < Duration::from_millis(300), "{after:?}");
    }

    #[test]
    fn an_o_to_t_rpi_alone_outside_the_range_is_refused() {
        let adapter = start(&config(&device(122))).unwrap();
        let mut o = Originator::connect(&adapter);
        let mut req = o.request(&open_for(4, 8));
        req.o_to_t_rpi_us = 500;
        let mut mr = vec![0x54, 0x02, 0x20, 0x06, 0x24, 0x01];
        mr.extend(req.encode());
        let r = o.encap.cip(o.session, &mr);
        eprintln!("PROBE O→T 500 us alone: {:?}", status_of(&r));
        assert_eq!(status_of(&r), (0x01, vec![0x0111]));
    }

    #[test]
    fn a_large_forward_open_carries_more_than_511_bytes() {
        let text = device(123)
            .replace("{ instance = 100, size = 8 }", "{ instance = 100, size = 600 }")
            .replace("{ instance = 150, size = 4 }", "{ instance = 150, size = 700 }");
        let adapter = start(&config(&text)).unwrap();
        let mut o = Originator::connect(&adapter);
        let SocketAddr::V4(me) = o.udp.local_addr().unwrap() else { panic!() };
        let sockaddr = ethernetip_core::cpf::encode_sockaddr_in_v4(SocketAddr::V4(me)).unwrap();
        let env = o.encap.rr(o.session, &large_fo(0x5555, 20_000, 20_000, 700 + 6, 600 + 2), vec![ethernetip_core::cpf::Item::new(0x8001, sockaddr)]);
        let r = env.find(0x00B2).unwrap().data.clone();
        eprintln!("PROBE large 700/600: {:02X?}", &r[..r.len().min(8)]);
        assert_eq!(&r[..4], &[0xDB, 0, 0, 0], "{r:02X?}");
        let mut buf = vec![0u8; 2000];
        let (n, _) = o.udp.recv_from(&mut buf).unwrap();
        let f = ethernetip_connections::decode_frame(&buf[..n], false).unwrap();
        assert_eq!(f.data.len(), 600);
        let to = env.find(0x8000).map(|i| SocketAddr::V4(ethernetip_core::cpf::decode_sockaddr_in_v4(&i.data).unwrap())).unwrap();
        let id = u32::from_le_bytes(r[4..8].try_into().unwrap());
        let mut data = vec![0u8; 700];
        data[0] = 1;
        data[2] = 0x21;
        data[3] = 0x43;
        o.send_output(to, id, 1, 1, true, &data);
        assert!(wait(Duration::from_secs(3), || adapter.get("setpoint").unwrap() == Value::U16(0x4321)));
    }

    #[test]
    fn an_equal_sequence_is_no_newer() {
        let adapter = start(&config(&device(124))).unwrap();
        let mut o = Originator::connect(&adapter);
        let (reply, to) = o.open(&adapter, &open_for(4, 8));
        let id = reply.o_to_t_connection_id;
        o.send_output(to, id, 10, 10, true, &[1, 0, 0x11, 0x11]);
        assert!(wait(Duration::from_secs(5), || adapter.get("setpoint").unwrap() == Value::U16(0x1111)));
        // The same 32-bit sequence again, with a new count and new data: no newer, so stale.
        o.send_output(to, id, 10, 11, true, &[1, 0, 0x22, 0x22]);
        std::thread::sleep(Duration::from_millis(200));
        let s = adapter.status();
        eprintln!("PROBE equal sequence: setpoint {:?}, stale {}, consumed {}", adapter.get("setpoint").unwrap(), s.stale_frames, s.connections[0].consumed);
        assert_eq!((adapter.get("setpoint").unwrap(), s.stale_frames), (Value::U16(0x1111), 1));
    }

    #[test]
    fn the_compatibility_bit_still_needs_the_major() {
        let adapter = start(&config(&device(125))).unwrap();
        let mut o = Originator::connect(&adapter);
        // The device is 2.5: compatible with major 3, minor 1 is not a revision it can emulate.
        let mut k = [0x34, 0x04, 0, 0, 0, 0, 0, 0, 0x83, 1];
        k[2..4].copy_from_slice(&9u16.to_le_bytes());
        k[4..6].copy_from_slice(&7u16.to_le_bytes());
        k[6..8].copy_from_slice(&3u16.to_le_bytes());
        let (r, _) = o.forward_open(&Open { key: Some(k), ..open_for(4, 8) });
        eprintln!("PROBE compat major 3: {:?}", status_of(&r));
        assert_eq!(status_of(&r), (0x01, vec![0x0116]));
    }

    #[test]
    fn the_first_frame_comes_at_once_at_a_long_rpi() {
        let adapter = start(&config(&device(126))).unwrap();
        let mut o = Originator::connect(&adapter);
        let opened = Instant::now();
        o.open(&adapter, &Open { rpi_us: 1_000_000, ..open_for(4, 8) });
        o.next_input(false);
        eprintln!("PROBE first frame at a 1 s RPI after {:?}", opened.elapsed());
        assert!(opened.elapsed() < Duration::from_millis(100), "{:?}", opened.elapsed());
    }

    #[test]
    fn stale_frames_do_not_feed_the_watchdog() {
        let adapter = start(&config(&device(127))).unwrap();
        let mut o = Originator::connect(&adapter);
        let (reply, to) = o.open(&adapter, &Open { rpi_us: 10_000, multiplier: 0, ..open_for(4, 8) });
        let id = reply.o_to_t_connection_id;
        o.send_output(to, id, 100, 1, true, &[0; 4]);
        std::thread::sleep(Duration::from_millis(5));
        // Only late frames from now on: the connection must time out about 40 ms after the last fresh one.
        let fresh = Instant::now();
        for i in 0..30u32 {
            o.send_output(to, id, 50 + i, 2, true, &[0; 4]);
            std::thread::sleep(Duration::from_millis(10));
            if adapter.status().timed_out == 1 {
                break;
            }
        }
        let timed_out = adapter.status().timed_out;
        eprintln!("PROBE stale only: timed_out {timed_out} after {:?}, stale {}", fresh.elapsed(), adapter.status().stale_frames);
        assert_eq!(timed_out, 1);
    }

    #[test]
    fn an_ipv6_bind_is_refused() {
        for bind in ["::1", "::"] {
            let r = start(&config(&format!("bind = \"{bind}\"\nport = 0\nio_port = 0\n")));
            assert_eq!(r.err().map(|e| e.to_string()), Some(format!("ethernetip.bind `{bind}`: EtherNet/IP is IPv4, a dotted quad")));
        }
    }

    #[test]
    fn a_port_key_or_data_segment_in_a_request_path_is_refused() {
        let adapter = start(&config(&device(128))).unwrap();
        let mut c = Encap::connect(adapter.addr);
        let s = c.register();
        // Get_Attribute_Single of the Identity's vendor through port 1, slot 5: the adapter routes nowhere.
        let r = c.cip(s, &[0x0E, 0x04, 0x01, 0x05, 0x20, 0x01, 0x24, 0x01, 0x30, 0x01]);
        // A port segment where only logical ones belong: 0x04 at its word, 0.
        assert_eq!(&r[..6], &[0x8E, 0x00, 0x04, 0x01, 0x00, 0x00], "{r:02X?}");
        // A key or data segment likewise, at the word it begins.
        let r = c.cip(s, &[0x0E, 0x02, 0x20, 0x01, 0x80, 0x00]);
        assert_eq!(&r[..6], &[0x8E, 0x00, 0x04, 0x01, 0x01, 0x00], "{r:02X?}");
    }

    #[test]
    fn a_misspelled_key_in_a_nested_table_is_refused() {
        for text in [
            "identity = { vendor = 9 }",
            "assemblies = [ { instance = 100, size = 8, sise = 9 } ]",
            "assemblies = [ { instance = 100, size = 8 } ]\nconnections = [ { config = 100, output = 100, input = 100, output_hedaer = \"modeless\" } ]",
            "assemblies = [ { instance = 100, size = 8 } ]\npoints = [ { name = \"a\", assembly = 100, byte = 0, tpye = \"u16\" } ]",
        ] {
            let r = toml::from_str::<EnipConfig>(text);
            eprintln!("PROBE nested misspelling {text:?}: {:?}", r.as_ref().map(|_| ()).map_err(|e| e.to_string().lines().last().unwrap_or("").to_string()));
            assert!(r.is_err(), "{text}");
        }
    }
}
