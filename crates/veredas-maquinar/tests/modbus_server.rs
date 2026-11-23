//! The Modbus server (contract: the Modbus server clause): a device's four tables on a real TCP
//! socket, met by a raw client speaking the protocol's own worked examples byte for byte and by a
//! client library the bench did not write, the exceptions where the protocol puts them, the
//! framing a stream brings (pipelined, split, foreign, malformed), the plant side by point and by
//! address, the knobs a client's failure paths need, and the port freed on stop.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

use tokio_modbus::client::{Reader, Writer};
use tokio_modbus::{ExceptionCode, Slave};
use veredas_maquinar::bench;
use veredas_maquinar::modbus::*;

fn config(text: &str) -> ModbusConfig {
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

/// A client that writes and reads bytes, nothing more: what is on the wire is what it asserts.
struct Raw {
    stream: TcpStream,
}

impl Raw {
    fn connect(addr: SocketAddr) -> Raw {
        let stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream.set_nodelay(true).unwrap();
        Raw { stream }
    }

    fn frame(transaction: u16, protocol: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&transaction.to_be_bytes());
        f.extend_from_slice(&protocol.to_be_bytes());
        f.extend_from_slice(&(pdu.len() as u16 + 1).to_be_bytes());
        f.push(unit);
        f.extend_from_slice(pdu);
        f
    }

    fn send(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).unwrap();
    }

    /// One answer: its header's fields and its PDU.
    fn read(&mut self) -> (u16, u16, u16, u8, Vec<u8>) {
        let mut h = [0u8; 7];
        self.stream.read_exact(&mut h).unwrap();
        let length = u16::from_be_bytes([h[4], h[5]]);
        let mut pdu = vec![0u8; length as usize - 1];
        self.stream.read_exact(&mut pdu).unwrap();
        (u16::from_be_bytes([h[0], h[1]]), u16::from_be_bytes([h[2], h[3]]), length, h[6], pdu)
    }

    /// Ask with a PDU and return the answer's PDU, the header checked: the transaction and the
    /// unit echoed, protocol 0, the length the PDU's plus the unit's byte.
    fn ask(&mut self, transaction: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
        self.send(&Raw::frame(transaction, 0, unit, pdu));
        let (t, p, l, u, answer) = self.read();
        assert_eq!((t, p, l as usize, u), (transaction, 0, answer.len() + 1, unit), "the answer's header to {pdu:02X?}");
        answer
    }

    /// Whether anything arrives within `d`: `None` for silence, `Some(0)` for the peer closing.
    fn arrives_within(&mut self, d: Duration) -> Option<usize> {
        self.stream.set_read_timeout(Some(d)).unwrap();
        let mut b = [0u8; 1];
        let got = match self.stream.read(&mut b) {
            Ok(n) => Some(n),
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => None,
            // A reset is the peer closing too.
            Err(_) => Some(0),
        };
        self.stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        got
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

#[test]
fn the_configuration_reads_from_toml_with_its_defaults_and_refuses_a_misspelled_key() {
    let c = config("");
    assert_eq!(c, ModbusConfig::default());
    assert_eq!((c.bind.as_str(), c.port, c.unit, c.max_connections), ("127.0.0.1", 502, None, None));
    assert_eq!((c.coils, c.discrete_inputs, c.input_registers, c.holding_registers), (None, None, None, None));
    assert!(c.points.is_empty());
    let c = config(
        "bind = \"127.0.0.2\"\nport = 0\nunit = 7\nmax_connections = 3\ncoils = 8\ndiscrete_inputs = 9\ninput_registers = 10\nholding_registers = 11\n\
         points = [ { name = \"speed\", table = \"holding-register\", address = 2, type = \"f32\", words = \"low-first\", value = \"1.5\" }, { name = \"run\", table = \"coil\", address = 0 } ]\n",
    );
    assert_eq!((c.bind.as_str(), c.port, c.unit, c.max_connections), ("127.0.0.2", 0, Some(7), Some(3)));
    assert_eq!((c.coils, c.discrete_inputs, c.input_registers, c.holding_registers), (Some(8), Some(9), Some(10), Some(11)));
    assert_eq!(
        c.points[0],
        PointConfig { name: "speed".into(), table: Table::HoldingRegister, address: 2, ty: Some(PointType::F32), words: Some(WordOrder::LowFirst), value: "1.5".into() }
    );
    assert_eq!(c.points[1], PointConfig { name: "run".into(), table: Table::Coil, address: 0, ty: None, words: None, value: String::new() });
    for bad in ["prot = 1", "points = [ { name = \"a\", table = \"coil\", address = 0, adress = 1 } ]"] {
        let e = toml::from_str::<ModbusConfig>(bad).unwrap_err().to_string();
        assert!(e.contains("unknown field"), "{bad}: {e}");
    }
    let e = toml::from_str::<ModbusConfig>("points = [ { name = \"a\", table = \"register\", address = 0 } ]").unwrap_err().to_string();
    assert!(e.contains("`coil`, `discrete-input`, `input-register`, `holding-register`"), "{e}");
    let e = toml::from_str::<ModbusConfig>("points = [ { name = \"a\", table = \"coil\", address = 0, type = \"u8\" } ]").unwrap_err().to_string();
    assert!(e.contains("`bool`, `u16`, `i16`, `u32`, `i32`, `f32`"), "{e}");
    let e = toml::from_str::<ModbusConfig>("points = [ { name = \"a\", table = \"coil\", address = 65536 } ]").unwrap_err().to_string();
    assert!(e.contains("65536"), "{e}");
}

#[test]
fn values_parse_per_type_and_nonsense_is_refused() {
    use PointType::*;
    assert_eq!(parse_value(Bool, ""), Ok(Value::Bool(false)));
    for t in ["true", "TRUE", "1", "on", "Yes"] {
        assert_eq!(parse_value(Bool, t), Ok(Value::Bool(true)), "{t}");
    }
    for t in ["false", "0", "off", "no"] {
        assert_eq!(parse_value(Bool, t), Ok(Value::Bool(false)), "{t}");
    }
    assert_eq!(parse_value(U16, "65535"), Ok(Value::U16(65535)));
    assert_eq!(parse_value(I16, "-32768"), Ok(Value::I16(-32768)));
    assert_eq!(parse_value(U32, "4294967295"), Ok(Value::U32(u32::MAX)));
    assert_eq!(parse_value(I32, " -7 "), Ok(Value::I32(-7)));
    assert_eq!(parse_value(F32, "1.5"), Ok(Value::F32(1.5)));
    assert_eq!(parse_value(U16, ""), Ok(Value::U16(0)));
    assert_eq!(parse_value(F32, ""), Ok(Value::F32(0.0)));
    assert_eq!(parse_value(Bool, "2"), Err("`2` is not a bool".into()));
    assert_eq!(parse_value(U16, "65536"), Err("`65536` is not a u16".into()));
    assert_eq!(parse_value(U16, "-1"), Err("`-1` is not a u16".into()));
    assert_eq!(parse_value(I16, "32768"), Err("`32768` is not a i16".into()));
    assert_eq!(parse_value(U32, "1.5"), Err("`1.5` is not a u32".into()));
    assert_eq!(parse_value(F32, "inf"), Err("`inf` is not a finite f32".into()));
    assert_eq!(parse_value(F32, "NaN"), Err("`NaN` is not a finite f32".into()));
    assert_eq!(parse_value(F32, "1e39"), Err("`1e39` is not a finite f32".into()));
    assert_eq!(parse_value(F32, "x"), Err("`x` is not an f32".into()));
}

#[test]
fn a_configuration_that_cannot_serve_is_refused_by_name_before_anything_binds() {
    let p = |rest: &str| format!("points = [ {rest} ]\n");
    assert_eq!(refused("bind = \" \""), "modbus.bind is empty");
    assert_eq!(refused("max_connections = 0"), "modbus.max_connections is 0: no client could connect");
    assert_eq!(refused("holding_registers = 65537"), "modbus.holding_registers is 65537, over the 65536 addresses a table can hold");
    assert!(validate(&config("coils = 65536\ndiscrete_inputs = 65536\ninput_registers = 65536\nholding_registers = 65536")).is_ok());
    assert_eq!(refused(&p("{ name = \"\", table = \"coil\", address = 0 }")), "modbus.points[0]: the point has no name");
    assert_eq!(refused(&p("{ name = \" a\", table = \"coil\", address = 0 }")), "modbus.points[0]: the name ` a` has surrounding whitespace");
    assert_eq!(
        refused(&p("{ name = \"a\", table = \"coil\", address = 0 }, { name = \"a\", table = \"coil\", address = 1 }")),
        "modbus.points[1]: two points are named `a`"
    );
    assert_eq!(
        refused(&p("{ name = \"a\", table = \"coil\", address = 0, type = \"u16\" }")),
        "modbus.points[0] (`a`): a coil holds a bool, not u16"
    );
    assert_eq!(
        refused(&p("{ name = \"a\", table = \"discrete-input\", address = 0, type = \"f32\" }")),
        "modbus.points[0] (`a`): a discrete-input holds a bool, not f32"
    );
    assert_eq!(
        refused(&p("{ name = \"a\", table = \"input-register\", address = 0, type = \"bool\" }")),
        "modbus.points[0] (`a`): a input-register holds a number (u16, i16, u32, i32 or f32), not a bool"
    );
    assert_eq!(
        refused(&p("{ name = \"a\", table = \"holding-register\", address = 0, words = \"high-first\" }")),
        "modbus.points[0] (`a`): `words` orders the two registers of a u32, i32 or f32, not a u16"
    );
    assert_eq!(
        refused(&p("{ name = \"a\", table = \"coil\", address = 0, words = \"low-first\" }")),
        "modbus.points[0] (`a`): `words` orders the two registers of a u32, i32 or f32, not a bool"
    );
    assert_eq!(
        refused(&p("{ name = \"a\", table = \"holding-register\", address = 65535, type = \"u32\" }")),
        "modbus.points[0] (`a`): a u32 at 65535 runs past the last address, 65535"
    );
    assert!(validate(&config(&p("{ name = \"a\", table = \"holding-register\", address = 65534, type = \"u32\" }, { name = \"b\", table = \"coil\", address = 65535 }"))).is_ok());
    assert_eq!(
        refused(&p("{ name = \"a\", table = \"holding-register\", address = 4, type = \"f32\" }, { name = \"b\", table = \"holding-register\", address = 5 }")),
        "modbus.points[1] (`b`): holding-register 5..5 overlaps point `a`"
    );
    assert_eq!(
        refused(&p("{ name = \"a\", table = \"holding-register\", address = 5 }, { name = \"b\", table = \"holding-register\", address = 4, type = \"i32\" }")),
        "modbus.points[1] (`b`): holding-register 4..5 overlaps point `a`"
    );
    // The same address in two tables is two places.
    assert!(validate(&config(&p("{ name = \"a\", table = \"holding-register\", address = 5 }, { name = \"b\", table = \"input-register\", address = 5 }, { name = \"c\", table = \"coil\", address = 5 }, { name = \"d\", table = \"discrete-input\", address = 5 }"))).is_ok());
    assert_eq!(
        refused(&format!("holding_registers = 5\n{}", p("{ name = \"a\", table = \"holding-register\", address = 4, type = \"u32\" }"))),
        "modbus.points: point `a` (u32 at 4) is past the end of the holding-register table (holding_registers = 5)"
    );
    assert!(validate(&config(&format!("holding_registers = 6\n{}", p("{ name = \"a\", table = \"holding-register\", address = 4, type = \"u32\" }")))).is_ok());
    assert_eq!(
        refused(&p("{ name = \"a\", table = \"holding-register\", address = 0, type = \"i16\", value = \"40000\" }")),
        "modbus.points[0] (`a`).value: `40000` is not a i16"
    );
    // Refused before anything binds: a port that is taken is never reached.
    let taken = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = taken.local_addr().unwrap().port();
    let e = start(&config(&format!("port = {port}\n{}", p("{ name = \"a\", table = \"coil\", address = 0, value = \"maybe\" }")))).unwrap_err();
    assert_eq!(e.to_string(), "modbus.points[0] (`a`).value: `maybe` is not a bool");
    let e = start(&config(&format!("port = {port}"))).unwrap_err();
    assert!(e.to_string().starts_with(&format!("bind 127.0.0.1:{port}: ")), "{e}");
}

/// Bits from the lowest address up, as the examples list them.
fn bits(s: &str) -> Vec<bool> {
    s.chars().filter(|c| !c.is_whitespace()).map(|c| c == '1').collect()
}

/// The protocol's own worked examples, request and answer byte for byte (the Modbus application
/// protocol specification's examples for each function code), against a server holding the values
/// they read.
#[test]
fn the_protocol_s_own_examples_answer_byte_for_byte() {
    let server = start(&config("port = 0\ncoils = 200\ndiscrete_inputs = 220\ninput_registers = 10\nholding_registers = 120\n")).unwrap();
    // Coils 20 to 38 (addresses 19 to 37): CD 6B 05, the lowest coil in the lowest bit.
    server.set_bits(Table::Coil, 19, &bits("10110011 11010110 101")).unwrap();
    // Discrete inputs 197 to 218 (addresses 196 to 217): AC DB 35.
    server.set_bits(Table::DiscreteInput, 196, &bits("00110101 11011011 101011")).unwrap();
    // Holding registers 108 to 110 (addresses 107 to 109).
    server.set_registers(Table::HoldingRegister, 107, &[0x022B, 0x0000, 0x0064]).unwrap();
    // Input register 9 (address 8).
    server.set_registers(Table::InputRegister, 8, &[0x000A]).unwrap();
    // The register the mask example changes holds 0x12; the ones its read-write example reads.
    server.set_registers(Table::HoldingRegister, 4, &[0x0012]).unwrap();
    let mut c = Raw::connect(server.addr);
    let examples: &[(&[u8], &[u8])] = &[
        (&[0x01, 0x00, 0x13, 0x00, 0x13], &[0x01, 0x03, 0xCD, 0x6B, 0x05]),
        (&[0x02, 0x00, 0xC4, 0x00, 0x16], &[0x02, 0x03, 0xAC, 0xDB, 0x35]),
        (&[0x03, 0x00, 0x6B, 0x00, 0x03], &[0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64]),
        (&[0x04, 0x00, 0x08, 0x00, 0x01], &[0x04, 0x02, 0x00, 0x0A]),
        (&[0x05, 0x00, 0xAC, 0xFF, 0x00], &[0x05, 0x00, 0xAC, 0xFF, 0x00]),
        (&[0x06, 0x00, 0x01, 0x00, 0x03], &[0x06, 0x00, 0x01, 0x00, 0x03]),
        (&[0x0F, 0x00, 0x13, 0x00, 0x0A, 0x02, 0xCD, 0x01], &[0x0F, 0x00, 0x13, 0x00, 0x0A]),
        (&[0x10, 0x00, 0x01, 0x00, 0x02, 0x04, 0x00, 0x0A, 0x01, 0x02], &[0x10, 0x00, 0x01, 0x00, 0x02]),
        (&[0x16, 0x00, 0x04, 0x00, 0xF2, 0x00, 0x25], &[0x16, 0x00, 0x04, 0x00, 0xF2, 0x00, 0x25]),
        // The exception example: coil 1186 does not exist.
        (&[0x01, 0x04, 0xA1, 0x00, 0x01], &[0x81, 0x02]),
    ];
    for (i, (request, answer)) in examples.iter().enumerate() {
        let tid = 0x0100 + i as u16;
        assert_eq!(c.ask(tid, 0x11, request), answer.to_vec(), "example {i}: {request:02X?}");
    }
    // What the writes left, read on the plant side.
    assert_eq!(server.bits(Table::Coil, 172, 1).unwrap(), [true]);
    assert_eq!(server.registers(Table::HoldingRegister, 1, 2).unwrap(), [0x000A, 0x0102], "the multiple write after the single one");
    assert_eq!(server.bits(Table::Coil, 19, 10).unwrap(), bits("10110011 10"));
    assert_eq!(server.bits(Table::Coil, 29, 9).unwrap(), bits("01011010 1"), "the coils past the written ten untouched");
    assert_eq!(server.registers(Table::HoldingRegister, 4, 1).unwrap(), [0x0017], "(0x12 AND 0xF2) OR (0x25 AND NOT 0xF2)");
    // Read and write in one request: the write lands first, then the read.
    server.set_registers(Table::HoldingRegister, 3, &[0x00FE, 0x0ACD, 0x0001, 0x0003, 0x000D, 0x00FF]).unwrap();
    let rw = [0x17, 0x00, 0x03, 0x00, 0x06, 0x00, 0x0E, 0x00, 0x03, 0x06, 0x00, 0xFF, 0x00, 0xFF, 0x00, 0xFF];
    assert_eq!(c.ask(0x0200, 0x11, &rw), [0x17, 0x0C, 0x00, 0xFE, 0x0A, 0xCD, 0x00, 0x01, 0x00, 0x03, 0x00, 0x0D, 0x00, 0xFF]);
    assert_eq!(server.registers(Table::HoldingRegister, 14, 3).unwrap(), [0x00FF; 3]);
    let overlapping = [0x17, 0x00, 0x03, 0x00, 0x02, 0x00, 0x04, 0x00, 0x01, 0x02, 0xBE, 0xEF];
    assert_eq!(c.ask(0x0201, 0x11, &overlapping), [0x17, 0x04, 0x00, 0xFE, 0xBE, 0xEF], "the read sees the write it overlaps");
    // A transaction id is any 16 bits, echoed; so is the unit.
    assert_eq!(c.ask(0xFFFF, 0xFF, &[0x04, 0x00, 0x08, 0x00, 0x01]), [0x04, 0x02, 0x00, 0x0A]);
    assert_eq!(c.ask(0x0000, 0x00, &[0x04, 0x00, 0x08, 0x00, 0x01]), [0x04, 0x02, 0x00, 0x0A]);
    let s = server.status();
    assert_eq!(s.answered.values().sum::<u64>(), 13);
    assert_eq!(s.exceptions, [((0x01, 0x02), 1)].into_iter().collect());
    assert_eq!(s.recent.len(), 14);
    assert_eq!(s.recent[0].request, examples[0].0.to_vec());
    assert_eq!(s.recent[0].response.as_deref(), Some(examples[0].1));
    assert_eq!((s.recent[0].transaction, s.recent[0].unit, s.recent[0].peer), (0x0100, 0x11, c.stream.local_addr().unwrap()));
}

#[test]
fn exceptions_come_where_the_protocol_puts_them() {
    let server = start(&config("port = 0\ncoils = 2000\ndiscrete_inputs = 0\ninput_registers = 125\nholding_registers = 130\n")).unwrap();
    let mut c = Raw::connect(server.addr);
    let mut tid = 0u16;
    let mut ask = |pdu: &[u8]| {
        tid += 1;
        c.ask(tid, 1, pdu)
    };
    // A function the server does not answer: 01, whatever follows it.
    for f in [0x00u8, 0x07, 0x08, 0x0B, 0x11, 0x14, 0x18, 0x2B, 0x41, 0x7F] {
        assert_eq!(ask(&[f, 0x00, 0x00, 0x00, 0x01]), [f | 0x80, 0x01], "function {f:#04x}");
    }
    assert_eq!(ask(&[0x2B, 0x0E, 0x01, 0x00]), [0xAB, 0x01]);
    assert_eq!(ask(&[0x81]), [0x81, 0x01], "an exception's code is no request");
    // The quantity a read takes: 1 to 2000 bits, 1 to 125 registers; the address after it.
    assert_eq!(ask(&[0x01, 0x00, 0x00, 0x07, 0xD0]).len(), 2 + 250, "2000 coils");
    assert_eq!(ask(&[0x01, 0x00, 0x00, 0x07, 0xD1]), [0x81, 0x03], "2001 coils");
    assert_eq!(ask(&[0x01, 0x00, 0x00, 0x00, 0x00]), [0x81, 0x03], "no coil");
    assert_eq!(ask(&[0x01, 0x00, 0x01, 0x07, 0xD0]), [0x81, 0x02], "2000 coils from 1 run past 2000");
    assert_eq!(ask(&[0x01, 0xFF, 0xFF, 0x07, 0xD1]), [0x81, 0x03], "the quantity before the address");
    assert_eq!(ask(&[0x02, 0x00, 0x00, 0x00, 0x01]), [0x82, 0x02], "an empty table has no address");
    assert_eq!(ask(&[0x04, 0x00, 0x00, 0x00, 0x7D]).len(), 2 + 250, "125 registers");
    assert_eq!(ask(&[0x04, 0x00, 0x00, 0x00, 0x7E]), [0x84, 0x03], "126 registers");
    assert_eq!(ask(&[0x03, 0x00, 0x05, 0x00, 0x7D]).len(), 2 + 250, "125 registers ending at the last one");
    assert_eq!(ask(&[0x03, 0x00, 0x06, 0x00, 0x7D]), [0x83, 0x02], "one past the last register");
    assert_eq!(ask(&[0x03, 0x00, 0x00, 0x00, 0x00]), [0x83, 0x03]);
    assert_eq!(ask(&[0x03, 0xFF, 0xFF, 0x00, 0x7D]), [0x83, 0x02], "an address near the top does not wrap");
    // A request of the wrong length for its function: 03.
    assert_eq!(ask(&[0x03, 0x00, 0x00, 0x00]), [0x83, 0x03]);
    assert_eq!(ask(&[0x03, 0x00, 0x00, 0x00, 0x01, 0x00]), [0x83, 0x03]);
    assert_eq!(ask(&[0x05, 0x00, 0x00, 0xFF]), [0x85, 0x03]);
    assert_eq!(ask(&[0x16, 0x00, 0x00, 0x00, 0x00, 0x00]), [0x96, 0x03]);
    // A single coil takes FF00 or 0000, its value checked before its address.
    assert_eq!(ask(&[0x05, 0x00, 0x00, 0x12, 0x34]), [0x85, 0x03]);
    assert_eq!(ask(&[0x05, 0x00, 0x00, 0x00, 0xFF]), [0x85, 0x03]);
    assert_eq!(ask(&[0x05, 0xFF, 0xFF, 0x12, 0x34]), [0x85, 0x03]);
    assert_eq!(ask(&[0x05, 0x07, 0xD0, 0xFF, 0x00]), [0x85, 0x02], "coil 2000 is past the table");
    assert_eq!(ask(&[0x06, 0x00, 0x82, 0x00, 0x01]), [0x86, 0x02], "register 130 is past the table");
    // Several coils: 1 to 1968, the byte count the quantity's, the bytes the count's.
    let coils = |n: u16, count: u8, bytes: usize| {
        let mut pdu = vec![0x0F, 0x00, 0x00];
        pdu.extend_from_slice(&n.to_be_bytes());
        pdu.push(count);
        pdu.extend(std::iter::repeat_n(0xAA, bytes));
        pdu
    };
    assert_eq!(ask(&coils(1968, 246, 246)), [0x0F, 0x00, 0x00, 0x07, 0xB0]);
    // Every coil set apart from what any request below writes (its bytes 0xAA), so a refused
    // request that wrote anyway shows.
    server.set_bits(Table::Coil, 0, &[true; 2000]).unwrap();
    assert_eq!(ask(&coils(1969, 247, 247)), [0x8F, 0x03]);
    assert_eq!(ask(&coils(0, 0, 0)), [0x8F, 0x03]);
    assert_eq!(ask(&coils(9, 1, 1)), [0x8F, 0x03], "nine coils need two bytes");
    assert_eq!(ask(&coils(9, 2, 1)), [0x8F, 0x03], "the bytes the count names are not there");
    assert_eq!(ask(&coils(9, 2, 3)), [0x8F, 0x03], "a byte more than the count");
    assert_eq!(ask(&[0x0F, 0x00, 0x00, 0x00]), [0x8F, 0x03]);
    let mut past = coils(1, 1, 1);
    past[1..3].copy_from_slice(&2000u16.to_be_bytes());
    assert_eq!(ask(&past), [0x8F, 0x02]);
    assert_eq!(server.bits(Table::Coil, 0, 2000).unwrap(), [true; 2000], "no refused write of coils wrote");
    // Several registers: 1 to 123, the byte count twice the quantity.
    let regs = |at: u16, n: u16, count: u8, bytes: usize| {
        let mut pdu = vec![0x10];
        pdu.extend_from_slice(&at.to_be_bytes());
        pdu.extend_from_slice(&n.to_be_bytes());
        pdu.push(count);
        pdu.extend(std::iter::repeat_n(0x01, bytes));
        pdu
    };
    assert_eq!(ask(&regs(0, 123, 246, 246)), [0x10, 0x00, 0x00, 0x00, 0x7B]);
    server.set_registers(Table::HoldingRegister, 0, &[0xBEEF; 130]).unwrap();
    // 124 registers cannot fit a frame (their PDU would be 254 bytes, one more than a frame holds):
    // asked in the largest frame there is, they are refused 03.
    assert_eq!(ask(&regs(0, 124, 248, 247)), [0x90, 0x03]);
    assert_eq!(ask(&regs(0, 2, 3, 3)), [0x90, 0x03]);
    assert_eq!(ask(&regs(0, 2, 4, 3)), [0x90, 0x03]);
    assert_eq!(ask(&regs(129, 2, 4, 4)), [0x90, 0x02]);
    assert_eq!(server.registers(Table::HoldingRegister, 0, 130).unwrap(), [0xBEEF; 130], "no refused write of registers wrote");
    assert_eq!(ask(&regs(129, 1, 2, 2)), [0x10, 0x00, 0x81, 0x00, 0x01]);
    assert_eq!(server.registers(Table::HoldingRegister, 129, 1).unwrap(), [0x0101]);
    // The mask on an address past the table.
    assert_eq!(ask(&[0x16, 0x00, 0x82, 0xFF, 0xFF, 0x00, 0x00]), [0x96, 0x02]);
    server.set_registers(Table::HoldingRegister, 0, &[0xBEEF; 2]).unwrap();
    assert_eq!(ask(&[0x16, 0x00, 0x00, 0x00, 0x00, 0x00]), [0x96, 0x03]);
    assert_eq!(server.registers(Table::HoldingRegister, 0, 1).unwrap(), [0xBEEF], "a refused mask masked nothing");
    // Read and write in one: 1 to 125 read, 1 to 121 written, either range past the table 02,
    // nothing written when either is refused.
    let rw = |read_at: u16, read_n: u16, write_at: u16, write_n: u16, count: u8, bytes: usize| {
        let mut pdu = vec![0x17];
        for v in [read_at, read_n, write_at, write_n] {
            pdu.extend_from_slice(&v.to_be_bytes());
        }
        pdu.push(count);
        pdu.extend(std::iter::repeat_n(0x07, bytes));
        pdu
    };
    assert_eq!(ask(&rw(0, 125, 0, 121, 242, 242)).len(), 2 + 250);
    server.set_registers(Table::HoldingRegister, 0, &[0xBEEF; 130]).unwrap();
    assert_eq!(ask(&rw(0, 126, 0, 1, 2, 2)), [0x97, 0x03]);
    assert_eq!(ask(&rw(0, 1, 0, 122, 244, 243)), [0x97, 0x03], "122 written, in the largest frame there is");
    assert_eq!(ask(&rw(0, 1, 0, 1, 3, 3)), [0x97, 0x03]);
    assert_eq!(server.registers(Table::HoldingRegister, 0, 130).unwrap(), [0xBEEF; 130], "no refused read-and-write wrote");
    server.set_registers(Table::HoldingRegister, 0, &[0x0000]).unwrap();
    assert_eq!(ask(&rw(130, 1, 0, 1, 2, 2)), [0x97, 0x02], "the read past the table");
    assert_eq!(ask(&rw(0, 1, 130, 1, 2, 2)), [0x97, 0x02], "the write past the table");
    assert_eq!(server.registers(Table::HoldingRegister, 0, 1).unwrap(), [0x0000], "neither refused request wrote");
    // Nothing a refused write names lands.
    server.set_registers(Table::HoldingRegister, 128, &[0xAAAA, 0xBBBB]).unwrap();
    assert_eq!(ask(&regs(128, 3, 6, 6)), [0x90, 0x02]);
    assert_eq!(server.registers(Table::HoldingRegister, 128, 2).unwrap(), [0xAAAA, 0xBBBB]);
    let s = server.status();
    assert_eq!(s.exceptions.get(&(0x01, 0x03)), Some(&3));
    assert_eq!(s.exceptions.get(&(0x2B, 0x01)), Some(&2));
    assert_eq!(s.exceptions.get(&(0x97, 0x03)), None, "counted under the request's function, not the answer's");
    assert_eq!(s.exceptions.get(&(0x17, 0x03)), Some(&3));
    assert_eq!(s.exceptions.get(&(0x16, 0x03)), Some(&2));
}

#[test]
fn a_unit_the_server_does_not_play_is_answered_as_a_gateway_answers() {
    let server = start(&config("port = 0\nunit = 1\nholding_registers = 2\n")).unwrap();
    let mut c = Raw::connect(server.addr);
    assert_eq!(c.ask(1, 1, &[0x03, 0x00, 0x00, 0x00, 0x02]), [0x03, 0x04, 0, 0, 0, 0]);
    for unit in [0u8, 2, 0xFF] {
        assert_eq!(c.ask(2, unit, &[0x03, 0x00, 0x00, 0x00, 0x02]), [0x83, 0x0B], "unit {unit}");
        assert_eq!(c.ask(3, unit, &[0x06, 0x00, 0x00, 0x00, 0x09]), [0x86, 0x0B], "unit {unit}");
    }
    assert_eq!(server.registers(Table::HoldingRegister, 0, 1).unwrap(), [0], "no write reached the unit");
    // Without `unit`, every unit is the device.
    let any = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
    let mut c = Raw::connect(any.addr);
    for unit in [0u8, 1, 2, 0xF7, 0xFF] {
        assert_eq!(c.ask(9, unit, &[0x03, 0x00, 0x00, 0x00, 0x01]), [0x03, 0x02, 0, 0], "unit {unit}");
    }
}

/// The plant's points: a number over registers in either word order, in every type.
fn device() -> ModbusConfig {
    config(
        "port = 0\nholding_registers = 20\n\
         points = [\n\
           { name = \"setpoint\", table = \"holding-register\", address = 0, type = \"f32\", value = \"1.5\" },\n\
           { name = \"setpoint_swapped\", table = \"holding-register\", address = 2, type = \"f32\", words = \"low-first\", value = \"-2.25\" },\n\
           { name = \"count\", table = \"holding-register\", address = 4, type = \"i32\", value = \"-100000\" },\n\
           { name = \"offset\", table = \"holding-register\", address = 6, type = \"i16\", value = \"-2\" },\n\
           { name = \"mode\", table = \"holding-register\", address = 7, value = \"3\" },\n\
           { name = \"total\", table = \"input-register\", address = 1, type = \"u32\", words = \"low-first\", value = \"305419896\" },\n\
           { name = \"speed\", table = \"input-register\", address = 0, value = \"1200\" },\n\
           { name = \"run\", table = \"coil\", address = 3 },\n\
           { name = \"ready\", table = \"discrete-input\", address = 9, value = \"true\" },\n\
         ]\n",
    )
}

#[test]
fn the_points_lie_in_their_tables_as_their_types_and_word_orders_say() {
    let server = start(&device()).unwrap();
    // The sizes a table takes without a key reach its last point; the given one stands.
    assert!(server.registers(Table::InputRegister, 0, 3).is_ok());
    assert_eq!(
        server.registers(Table::InputRegister, 0, 4).unwrap_err().to_string(),
        "input-register 0..4 is past the table's end (3 address(es))"
    );
    assert_eq!(server.bits(Table::Coil, 0, 4).unwrap(), [false; 4]);
    assert!(server.bits(Table::Coil, 0, 5).is_err());
    assert_eq!(server.bits(Table::DiscreteInput, 0, 10).unwrap(), bits("0000000001"));
    assert!(server.registers(Table::HoldingRegister, 0, 20).is_ok());
    assert!(server.registers(Table::HoldingRegister, 0, 21).is_err());
    assert_eq!(server.bits(Table::HoldingRegister, 0, 1).unwrap_err().to_string(), "the holding-register table holds registers");
    assert_eq!(server.registers(Table::Coil, 0, 1).unwrap_err().to_string(), "the coil table holds bits");
    // The registers as the wire carries them.
    let f = 1.5f32.to_bits();
    let g = (-2.25f32).to_bits();
    let n = -100000i32 as u32;
    assert_eq!(
        server.registers(Table::HoldingRegister, 0, 8).unwrap(),
        [(f >> 16) as u16, f as u16, g as u16, (g >> 16) as u16, (n >> 16) as u16, n as u16, (-2i16) as u16, 3]
    );
    assert_eq!(server.registers(Table::InputRegister, 0, 3).unwrap(), [1200, 0x5678, 0x1234]);
    // Read back by point.
    assert_eq!(
        server.snapshot(),
        [
            ("setpoint".to_string(), Value::F32(1.5)),
            ("setpoint_swapped".into(), Value::F32(-2.25)),
            ("count".into(), Value::I32(-100000)),
            ("offset".into(), Value::I16(-2)),
            ("mode".into(), Value::U16(3)),
            ("total".into(), Value::U32(0x12345678)),
            ("speed".into(), Value::U16(1200)),
            ("run".into(), Value::Bool(false)),
            ("ready".into(), Value::Bool(true)),
        ]
    );
    assert_eq!(server.point_names()[0], "setpoint");
    // Read as text, as a monitor shows a value.
    let text = |name: &str| server.get(name).unwrap().to_string();
    assert_eq!(
        [text("setpoint"), text("setpoint_swapped"), text("count"), text("offset"), text("mode"), text("total"), text("ready"), text("run")],
        ["1.5", "-2.25", "-100000", "-2", "3", "305419896", "true", "false"]
    );
    // Set by point, by value or by text; a type or a name that is not the point's refused.
    server.set_value("total", Value::U32(0xCAFEF00D)).unwrap();
    assert_eq!(server.registers(Table::InputRegister, 1, 2).unwrap(), [0xF00D, 0xCAFE]);
    server.set("count", "2147483647").unwrap();
    assert_eq!(server.registers(Table::HoldingRegister, 4, 2).unwrap(), [0x7FFF, 0xFFFF]);
    assert_eq!(server.set_value("count", Value::U32(1)).unwrap_err().to_string(), "point `count` is i32, not u32");
    assert_eq!(server.set("count", "x").unwrap_err().to_string(), "point `count`: `x` is not a i32");
    assert_eq!(server.set("nothing", "1").unwrap_err().to_string(), "no point `nothing`");
    assert_eq!(server.get("nothing").unwrap_err().to_string(), "no point `nothing`");
    // A register written raw is what the point reads.
    server.set_registers(Table::HoldingRegister, 0, &[0x4049, 0x0FDB]).unwrap();
    assert_eq!(server.get("setpoint").unwrap(), Value::F32(f32::from_bits(0x40490FDB)));
    server.set_registers(Table::HoldingRegister, 2, &[0x0000, 0x7FC0]).unwrap();
    match server.get("setpoint_swapped").unwrap() {
        Value::F32(v) => assert!(v.is_nan(), "{v}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(server.get("setpoint_swapped").unwrap().to_string(), "NaN");
    // A count past every address is refused, never added past what a number holds.
    assert_eq!(
        server.registers(Table::HoldingRegister, 1, usize::MAX).unwrap_err().to_string(),
        format!("holding-register 1..{} is past the table's end (20 address(es))", usize::MAX as u128 + 1)
    );
    assert!(server.bits(Table::Coil, 3, usize::MAX).is_err());
    assert!(server.set_registers(Table::HoldingRegister, 0, &[]).is_ok(), "nothing written is in range");
}

#[test]
fn a_client_the_bench_did_not_write_reads_and_writes_every_table() {
    let server = start(&device()).unwrap();
    let addr = server.addr;
    runtime().block_on(async {
        let mut ctx = tokio_modbus::client::tcp::connect_slave(addr, Slave(1)).await.unwrap();
        let words = ctx.read_holding_registers(0, 8).await.unwrap().unwrap();
        // The client's own reading of the words: the high-first single, the low-first one.
        assert_eq!(f32::from_bits(((words[0] as u32) << 16) | words[1] as u32), 1.5);
        assert_eq!(f32::from_bits(((words[3] as u32) << 16) | words[2] as u32), -2.25);
        assert_eq!((((words[4] as u32) << 16) | words[5] as u32) as i32, -100000);
        assert_eq!((words[6] as i16, words[7]), (-2, 3));
        assert_eq!(ctx.read_input_registers(0, 3).await.unwrap().unwrap(), [1200, 0x5678, 0x1234]);
        assert_eq!(ctx.read_coils(0, 4).await.unwrap().unwrap(), [false; 4]);
        assert_eq!(ctx.read_discrete_inputs(8, 2).await.unwrap().unwrap(), [false, true]);
        // Writes land where the plant side reads them, by point.
        let v = 12.75f32.to_bits();
        ctx.write_multiple_registers(0, &[(v >> 16) as u16, v as u16]).await.unwrap().unwrap();
        assert_eq!(server.get("setpoint").unwrap(), Value::F32(12.75));
        ctx.write_single_register(7, 0x12A9).await.unwrap().unwrap();
        assert_eq!(server.get("mode").unwrap(), Value::U16(0x12A9));
        ctx.write_single_coil(3, true).await.unwrap().unwrap();
        assert_eq!(server.get("run").unwrap(), Value::Bool(true));
        ctx.write_multiple_coils(0, &[true, false, true, false]).await.unwrap().unwrap();
        assert_eq!(server.bits(Table::Coil, 0, 4).unwrap(), [true, false, true, false]);
        ctx.masked_write_register(7, 0x00F0, 0x0005).await.unwrap().unwrap();
        // (0x12A9 AND 0x00F0) OR (0x0005 AND NOT 0x00F0): the masked bits kept, the others the OR's.
        assert_eq!(server.get("mode").unwrap(), Value::U16(0x00A5));
        let read = ctx.read_write_multiple_registers(6, 2, 6, &[0xFFFF, 0x0001]).await.unwrap().unwrap();
        assert_eq!(read, [0xFFFF, 0x0001]);
        assert_eq!((server.get("offset").unwrap(), server.get("mode").unwrap()), (Value::I16(-1), Value::U16(1)));
        // The plant side sets; the client reads it next.
        server.set_value("speed", Value::U16(1500)).unwrap();
        server.set("ready", "false").unwrap();
        server.set_value("setpoint_swapped", Value::F32(0.5)).unwrap();
        assert_eq!(ctx.read_input_registers(0, 1).await.unwrap().unwrap(), [1500]);
        assert_eq!(ctx.read_discrete_inputs(9, 1).await.unwrap().unwrap(), [false]);
        let w = ctx.read_holding_registers(2, 2).await.unwrap().unwrap();
        assert_eq!(f32::from_bits(((w[1] as u32) << 16) | w[0] as u32), 0.5);
        // The exceptions as the client reads them.
        assert_eq!(ctx.read_holding_registers(19, 2).await.unwrap(), Err(ExceptionCode::IllegalDataAddress));
        assert_eq!(ctx.read_discrete_inputs(10, 1).await.unwrap(), Err(ExceptionCode::IllegalDataAddress));
        assert_eq!(ctx.write_single_register(20, 1).await.unwrap(), Err(ExceptionCode::IllegalDataAddress));
        // A connection survives its exceptions.
        assert_eq!(ctx.read_input_registers(0, 1).await.unwrap().unwrap(), [1500]);
    });
    let s = server.status();
    assert!(s.answered.get(&0x03).is_some_and(|n| *n >= 2) && s.answered.get(&0x17) == Some(&1), "{s:?}");
}

#[test]
fn pipelined_and_split_requests_are_answered_in_order_and_whole() {
    let server = start(&config("port = 0\nholding_registers = 4\n")).unwrap();
    server.set_registers(Table::HoldingRegister, 0, &[10, 20, 30, 40]).unwrap();
    let mut c = Raw::connect(server.addr);
    // Three requests in one write: three answers, in order, each with its transaction.
    let mut burst = Vec::new();
    for (t, at) in [(7u16, 0u8), (8, 1), (9, 3)] {
        burst.extend(Raw::frame(t, 0, 1, &[0x03, 0x00, at, 0x00, 0x01]));
    }
    c.send(&burst);
    for (t, v) in [(7u16, 10u8), (8, 20), (9, 40)] {
        let (tid, _, _, _, pdu) = c.read();
        assert_eq!((tid, pdu), (t, vec![0x03, 0x02, 0x00, v]));
    }
    // One request a byte at a time: one answer, once the last byte is in.
    let frame = Raw::frame(11, 0, 1, &[0x06, 0x00, 0x02, 0x01, 0x02]);
    for (i, b) in frame.iter().enumerate() {
        c.send(&[*b]);
        if i + 1 < frame.len() {
            std::thread::sleep(Duration::from_millis(3));
        }
    }
    assert_eq!(c.read(), (11, 0, 6, 1, vec![0x06, 0x00, 0x02, 0x01, 0x02]));
    assert_eq!(c.arrives_within(Duration::from_millis(100)), None, "one answer, not more");
    // Requests in flight on several connections at once.
    let mut clients: Vec<Raw> = (0..5).map(|_| Raw::connect(server.addr)).collect();
    for (i, c) in clients.iter_mut().enumerate() {
        c.send(&Raw::frame(i as u16, 0, 1, &[0x06, 0x00, 0x03, 0x00, i as u8]));
    }
    for (i, c) in clients.iter_mut().enumerate() {
        assert_eq!(c.read().4, [0x06, 0x00, 0x03, 0x00, i as u8]);
    }
    assert!(wait(Duration::from_secs(5), || server.status().connections == 6));
}

#[test]
fn a_frame_of_another_protocol_is_dropped_and_a_length_no_frame_has_closes_the_connection() {
    let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
    let mut c = Raw::connect(server.addr);
    c.send(&Raw::frame(1, 1, 1, &[0x03, 0x00, 0x00, 0x00, 0x01]));
    c.send(&Raw::frame(2, 0xFFFF, 1, &[0x06, 0x00, 0x00, 0x00, 0x09]));
    assert_eq!(c.arrives_within(Duration::from_millis(300)), None, "no answer to another protocol");
    assert_eq!(server.registers(Table::HoldingRegister, 0, 1).unwrap(), [0], "and nothing it asked done");
    assert_eq!(c.ask(3, 1, &[0x03, 0x00, 0x00, 0x00, 0x01]), [0x03, 0x02, 0x00, 0x00], "the connection reads on");
    assert_eq!(server.status().dropped, 2);
    // A length below the unit and a function, or above a whole PDU's: the stream cannot be read
    // on, and the connection is closed.
    for length in [0u16, 1, 255, 0xFFFF] {
        let mut c = Raw::connect(server.addr);
        let mut f = vec![0x00, 0x01, 0x00, 0x00];
        f.extend_from_slice(&length.to_be_bytes());
        f.extend_from_slice(&[0x01, 0x03, 0x00, 0x00, 0x00, 0x01]);
        c.send(&f);
        assert_eq!(c.arrives_within(Duration::from_secs(5)), Some(0), "length {length}: closed");
    }
    // The longest frame there is, 254: a write of 123 registers.
    let mut c = Raw::connect(server.addr);
    let mut pdu = vec![0x10, 0x00, 0x00, 0x00, 0x7B, 0xF6];
    pdu.extend(std::iter::repeat_n(0u8, 246));
    assert_eq!(Raw::frame(1, 0, 1, &pdu).len(), 7 + 252);
    let mut pdu253 = pdu.clone();
    pdu253.push(0);
    assert_eq!(c.ask(1, 1, &pdu253), [0x90, 0x03], "253 bytes of PDU, length 254: read whole and answered");
    assert_eq!(c.ask(2, 1, &[0x03, 0x00, 0x00, 0x00, 0x01]), [0x03, 0x02, 0x00, 0x00]);
    let s = server.status();
    assert_eq!(s.malformed, 4);
    assert!(wait(Duration::from_secs(5), || server.status().connections == 2), "{:?}", server.status());
}

#[test]
fn the_knobs_delay_silence_and_drop_every_connection() {
    let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
    let mut c = Raw::connect(server.addr);
    let read = [0x03, 0x00, 0x00, 0x00, 0x01];
    // Late: every answer held for the delay, then on time again.
    server.set_delay(Duration::from_millis(300));
    let asked = Instant::now();
    assert_eq!(c.ask(1, 1, &read), [0x03, 0x02, 0x00, 0x00]);
    let late = asked.elapsed();
    assert!(late >= Duration::from_millis(300), "{late:?}");
    server.set_delay(Duration::ZERO);
    let asked = Instant::now();
    c.ask(2, 1, &read);
    assert!(asked.elapsed() < late, "{:?} after a delay of {late:?}", asked.elapsed());
    // Every answer held, not the next one alone: two asked in turn, then two pipelined (answered one
    // after the other, as a device serves a connection).
    server.set_delay(Duration::from_millis(300));
    for t in [10u16, 11] {
        let asked = Instant::now();
        c.ask(t, 1, &read);
        assert!(asked.elapsed() >= Duration::from_millis(300), "answer {t}: {:?}", asked.elapsed());
    }
    let asked = Instant::now();
    c.send(&[Raw::frame(12, 0, 1, &read), Raw::frame(13, 0, 1, &read)].concat());
    assert_eq!(c.read().0, 12);
    assert!(asked.elapsed() >= Duration::from_millis(300), "{:?}", asked.elapsed());
    assert_eq!(c.read().0, 13);
    assert!(asked.elapsed() >= Duration::from_millis(600), "the second after the first: {:?}", asked.elapsed());
    // Gone dumb while an answer waits out its delay: that answer is never sent either.
    server.set_delay(Duration::from_millis(500));
    c.send(&Raw::frame(14, 0, 1, &read));
    assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 14)));
    let answered = server.status().answered.get(&0x03).copied();
    server.set_silent(true);
    assert_eq!(c.arrives_within(Duration::from_millis(1000)), None, "the waiting answer withheld");
    let s = server.status();
    assert_eq!((s.unanswered, s.recent.back().unwrap().transaction, s.recent.back().unwrap().response.clone()), (1, 14, None));
    assert_eq!(s.answered.get(&0x03).copied(), answered, "a withheld answer is no answer");
    server.set_silent(false);
    // Silence for a moment inside the delay, over before the delay is: the answer is withheld too.
    c.send(&Raw::frame(16, 0, 1, &read));
    assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 16)));
    server.set_silent(true);
    std::thread::sleep(Duration::from_millis(50));
    server.set_silent(false);
    assert_eq!(c.arrives_within(Duration::from_millis(1000)), None, "silence that came and went still withheld it");
    let s = server.status();
    assert_eq!((s.unanswered, s.answered.get(&0x03).copied()), (2, answered));
    server.set_delay(Duration::ZERO);
    assert_eq!(c.ask(15, 1, &read), [0x03, 0x02, 0x00, 0x00], "the next answer is the next request's");
    // Silent: read, never answered, the connection up; answered again after, the silent ones never.
    server.set_silent(true);
    c.send(&Raw::frame(3, 0, 1, &read));
    c.send(&Raw::frame(4, 0, 1, &[0x06, 0x00, 0x00, 0x00, 0x05]));
    assert_eq!(c.arrives_within(Duration::from_millis(500)), None);
    assert!(wait(Duration::from_secs(5), || server.status().unanswered == 4));
    assert_eq!(server.registers(Table::HoldingRegister, 0, 1).unwrap(), [0], "a silent device does nothing it is asked");
    let s = server.status();
    assert_eq!((s.recent.back().unwrap().transaction, s.recent.back().unwrap().response.clone()), (4, None));
    server.set_silent(false);
    assert_eq!(c.ask(5, 1, &read), [0x03, 0x02, 0x00, 0x00], "the next answer is the next request's");
    // Every connection dropped: each client sees its end, the server listens on.
    let mut d = Raw::connect(server.addr);
    d.ask(1, 1, &read);
    assert!(wait(Duration::from_secs(5), || server.status().connections == 2));
    server.drop_connections();
    assert_eq!(server.status().connections, 0, "drop_connections returns with them closed");
    assert_eq!(c.arrives_within(Duration::from_secs(5)), Some(0));
    assert_eq!(d.arrives_within(Duration::from_secs(5)), Some(0));
    let mut e = Raw::connect(server.addr);
    assert_eq!(e.ask(6, 1, &read), [0x03, 0x02, 0x00, 0x00]);
    // A connection held while its answer waits out a delay is dropped as well.
    server.set_delay(Duration::from_secs(30));
    e.send(&Raw::frame(7, 0, 1, &read));
    assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 7)));
    let before = server.status().answered.get(&0x03).copied();
    let dropped = Instant::now();
    server.drop_connections();
    assert_eq!(e.arrives_within(Duration::from_secs(5)), Some(0));
    assert!(dropped.elapsed() < Duration::from_secs(5));
    // An answer its connection never sent is not counted, and the exchange shows none.
    let s = server.status();
    assert_eq!(s.answered.get(&0x03).copied(), before);
    assert_eq!((s.recent.back().unwrap().transaction, s.recent.back().unwrap().response.clone()), (7, None));
}

#[test]
fn a_connection_the_server_has_not_yet_taken_is_dropped_with_the_rest() {
    let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
    // A client's connect returns once the kernel has the connection, before the server takes it:
    // dropped at once, every time, never answered after.
    for _ in 0..50 {
        let mut fresh = Raw::connect(server.addr);
        server.drop_connections();
        fresh.send(&Raw::frame(1, 0, 1, &[0x03, 0x00, 0x00, 0x00, 0x01]));
        assert_eq!(fresh.arrives_within(Duration::from_secs(5)), Some(0), "closed, not answered");
    }
    assert_eq!(server.status().connections, 0);
    let mut after = Raw::connect(server.addr);
    assert_eq!(after.ask(2, 1, &[0x03, 0x00, 0x00, 0x00, 0x01]), [0x03, 0x02, 0x00, 0x00], "the next one is served");
}

#[test]
fn a_client_that_reads_nothing_is_dropped_at_once() {
    let server = start(&config("port = 0\nholding_registers = 125\n")).unwrap();
    let mut c = Raw::connect(server.addr);
    // Requests for 125 registers, written until the socket takes no more: the answers back up until
    // the server's write waits on a client that does not read.
    c.stream.set_nonblocking(true).unwrap();
    // A hundred requests a write, the place kept when the socket takes part of them, so no frame is
    // cut and a loaded machine still fills the buffers.
    let requests = Raw::frame(1, 0, 1, &[0x03, 0x00, 0x00, 0x00, 0x7D]).repeat(100);
    let mut at = 0;
    let started = Instant::now();
    let mut blocked = 0;
    while blocked < 200 && started.elapsed() < Duration::from_secs(60) {
        match c.stream.write(&requests[at..]) {
            Ok(n) => {
                at = (at + n) % requests.len();
                blocked = 0;
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                blocked += 1;
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(e) => panic!("{e}"),
        }
    }
    assert_eq!(blocked, 200, "the client's sends stopped being taken");
    let answered = server.status().answered.get(&0x03).copied().unwrap_or(0);
    assert!(answered > 0, "the server answered until its writes backed up");
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(server.status().answered.get(&0x03).copied().unwrap_or(0), answered, "and answers no more: its write waits");
    let dropped = Instant::now();
    server.drop_connections();
    assert!(dropped.elapsed() < Duration::from_secs(1), "{:?}", dropped.elapsed());
    assert_eq!(server.status().connections, 0);
    // The client meets the end once it reads what came before it.
    c.stream.set_nonblocking(false).unwrap();
    c.stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut buf = vec![0u8; 1 << 16];
    let reading = Instant::now();
    let ended = loop {
        if reading.elapsed() > Duration::from_secs(10) {
            break false;
        }
        match c.stream.read(&mut buf) {
            Ok(0) => break true,
            Ok(_) => continue,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => break false,
            Err(_) => break true,
        }
    };
    assert!(ended, "the connection ended");
}

#[test]
fn the_status_keeps_the_last_hundred_exchanges() {
    let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
    let mut c = Raw::connect(server.addr);
    for t in 0..150u16 {
        c.ask(t, 1, &[0x03, 0x00, 0x00, 0x00, 0x01]);
    }
    let s = server.status();
    assert_eq!(s.recent.len(), RECENT);
    assert_eq!((s.recent.front().unwrap().transaction, s.recent.back().unwrap().transaction), (50, 149));
    assert_eq!(s.answered.get(&0x03), Some(&150));
}

#[test]
fn connections_past_the_limit_are_closed_at_once_and_counted() {
    let server = start(&config("port = 0\nmax_connections = 2\nholding_registers = 1\n")).unwrap();
    let read = [0x03, 0x00, 0x00, 0x00, 0x01];
    let mut a = Raw::connect(server.addr);
    let mut b = Raw::connect(server.addr);
    a.ask(1, 1, &read);
    b.ask(1, 1, &read);
    let mut third = Raw::connect(server.addr);
    assert_eq!(third.arrives_within(Duration::from_secs(5)), Some(0), "closed at once");
    let s = server.status();
    assert_eq!((s.connections, s.accepted, s.refused), (2, 3, 1));
    a.ask(2, 1, &read);
    b.ask(2, 1, &read);
    drop(a);
    assert!(wait(Duration::from_secs(5), || server.status().connections == 1));
    let mut fourth = Raw::connect(server.addr);
    assert_eq!(fourth.ask(1, 1, &read), [0x03, 0x02, 0x00, 0x00]);
}

#[test]
fn stopping_closes_every_connection_and_frees_the_port() {
    let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
    let port = server.port;
    assert_eq!(server.addr, SocketAddr::from(([127, 0, 0, 1], port)));
    let mut c = Raw::connect(server.addr);
    c.ask(1, 1, &[0x03, 0x00, 0x00, 0x00, 0x01]);
    server.stop();
    assert_eq!(c.arrives_within(Duration::from_secs(5)), Some(0));
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err(), "nobody listens");
    let again = start(&config(&format!("port = {port}\nholding_registers = 1\n"))).unwrap();
    assert_eq!(again.port, port, "the port is free for the next start");
    // While it runs the port is the server's.
    let e = start(&config(&format!("port = {port}"))).unwrap_err();
    assert!(matches!(e, ModbusError::Bind { .. }), "{e}");
    drop(again);
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok(), "dropping the handle frees it too");
}

fn doc_block(n: usize) -> String {
    let text = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/bench-schema.md")).unwrap().replace("\r\n", "\n");
    let mut rest = text.as_str();
    let mut found = Vec::new();
    while let Some(start) = rest.find("```toml\n") {
        let body = &rest[start + "```toml\n".len()..];
        let end = body.find("```").unwrap();
        found.push(body[..end].to_string());
        rest = &body[end + 3..];
    }
    found.swap_remove(n)
}

#[test]
fn the_documented_drive_runs_from_its_bench_file() {
    let text = doc_block(1).replace("port = 5502", "port = 0");
    let spec = bench::load(&text, Path::new(".")).unwrap();
    assert_eq!(spec.plan(), ["modbus `drive`: 127.0.0.1:0 answering unit 1 with 4 point(s)"]);
    let plant = bench::Bench::start(spec).unwrap();
    let drive = plant.modbus("drive").unwrap();
    let addr = drive.addr;
    runtime().block_on(async {
        let mut ctx = tokio_modbus::client::tcp::connect_slave(addr, Slave(1)).await.unwrap();
        // A client commands a speed and starts the drive; the plant side sees both.
        ctx.write_single_register(0, 1450).await.unwrap().unwrap();
        ctx.write_single_coil(0, true).await.unwrap().unwrap();
        // The plant side answers with the speed reached and a ready input.
        drive.set_value("speed", Value::F32(1449.5)).unwrap();
        let w = ctx.read_input_registers(0, 2).await.unwrap().unwrap();
        assert_eq!(f32::from_bits(((w[0] as u32) << 16) | w[1] as u32), 1449.5);
        assert_eq!(ctx.read_discrete_inputs(0, 1).await.unwrap().unwrap(), [true]);
    });
    assert_eq!(drive.get("setpoint").unwrap(), Value::U16(1450));
    assert_eq!(drive.get("run").unwrap(), Value::Bool(true));
    let lines = plant.lines();
    assert_eq!(lines.len(), 1);
    assert!(lines[0].starts_with(&format!("modbus drive: {addr} ")), "{lines:?}");
    assert!(lines[0].contains("request(s) answered, 0 exception(s), 0 unanswered"), "{lines:?}");
    plant.stop();
    assert!(std::net::TcpListener::bind(addr).is_ok());
}

/// C-110's third refute round: the critic's probes, kept as pins (an answer counted as its write
/// begins and taken back when it fails or a drop cuts it; silence withholding what is not yet
/// handed to the socket; 03 before 02 and for every length and zero quantity; a refused 05; the
/// delay on exceptions; interleaved exchanges).
mod round_3 {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::time::{Duration, Instant};

    use veredas_maquinar::modbus::*;

    fn config(text: &str) -> ModbusConfig {
        toml::from_str(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
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

    struct Raw {
        stream: TcpStream,
    }

    impl Raw {
        fn connect(addr: SocketAddr) -> Raw {
            let stream = TcpStream::connect(addr).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            stream.set_nodelay(true).unwrap();
            Raw { stream }
        }

        fn frame(transaction: u16, protocol: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
            let mut f = Vec::new();
            f.extend_from_slice(&transaction.to_be_bytes());
            f.extend_from_slice(&protocol.to_be_bytes());
            f.extend_from_slice(&(pdu.len() as u16 + 1).to_be_bytes());
            f.push(unit);
            f.extend_from_slice(pdu);
            f
        }

        fn send(&mut self, bytes: &[u8]) {
            self.stream.write_all(bytes).unwrap();
        }

        fn read(&mut self) -> (u16, u16, u16, u8, Vec<u8>) {
            let mut h = [0u8; 7];
            self.stream.read_exact(&mut h).unwrap();
            let length = u16::from_be_bytes([h[4], h[5]]);
            let mut pdu = vec![0u8; length as usize - 1];
            self.stream.read_exact(&mut pdu).unwrap();
            (u16::from_be_bytes([h[0], h[1]]), u16::from_be_bytes([h[2], h[3]]), length, h[6], pdu)
        }

        fn ask(&mut self, transaction: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
            self.send(&Raw::frame(transaction, 0, unit, pdu));
            let (t, p, l, u, answer) = self.read();
            assert_eq!((t, p, l as usize, u), (transaction, 0, answer.len() + 1, unit), "the answer's header to {pdu:02X?}");
            answer
        }
    }

    /// Pipelines 125-register reads (each its own transaction id, from 0) until the client's sends are
    /// no longer taken: the server's write then waits on a client that reads nothing.
    fn fill(c: &mut Raw) -> u16 {
        c.stream.set_nonblocking(true).unwrap();
        let mut next: u16 = 0;
        let mut pending: Vec<u8> = Vec::new();
        let started = Instant::now();
        let mut blocked = 0;
        while blocked < 200 && started.elapsed() < Duration::from_secs(60) {
            if pending.is_empty() {
                for _ in 0..100 {
                    pending.extend(Raw::frame(next, 0, 1, &[0x03, 0x00, 0x00, 0x00, 0x7D]));
                    next = next.wrapping_add(1);
                }
            }
            match c.stream.write(&pending) {
                Ok(n) => {
                    pending.drain(..n);
                    blocked = 0;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    blocked += 1;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("{e}"),
            }
        }
        assert_eq!(blocked, 200, "the client's sends stopped being taken");
        next
    }

    /// G1a. "an answer counted when it is sent": a client resets its connection while its answer waits
    /// out a delay; the server's write then fails and the answer is never sent, yet it is counted as
    /// answered and its exchange shows it.
    #[test]
    fn an_answer_whose_write_fails_is_taken_back() {
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        server.set_delay(Duration::from_millis(300));
        let mut c = Raw::connect(server.addr);
        c.send(&Raw::frame(1, 0, 1, &[0x03, 0x00, 0x00, 0x00, 0x01]));
        assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 1)));
        // The client goes away with a reset, before the answer is sent.
        socket2::SockRef::from(&c.stream).set_linger(Some(Duration::ZERO)).unwrap();
        drop(c);
        // The server's task ends once the delay is over and its write fails.
        assert!(wait(Duration::from_secs(5), || server.status().connections == 0), "{:?}", server.status());
        let s = server.status();
        assert_eq!(s.answered.get(&0x03), None, "an answer never sent counted as answered");
        assert_eq!(s.recent.back().unwrap().response, None, "its exchange shows an answer never sent");
    }

    /// G1b. "an answer counted when it is sent ... one whose connection was dropped first, counted as
    /// none", "each showing its answer once sent": the answer whose write waits on a client that reads
    /// nothing is counted and shown before it is sent, and stays so when drop_connections closes its
    /// connection first.
    #[test]
    fn an_answer_is_counted_as_its_write_begins_and_taken_back_when_a_drop_cuts_it() {
        let server = start(&config("port = 0\nholding_registers = 125\n")).unwrap();
        let mut c = Raw::connect(server.addr);
        fill(&mut c);
        // Stable: the server reads no more requests, so its last exchange's answer is the one whose
        // write waits (nothing else awaits on a connection with no delay).
        let mut s = server.status();
        assert!(wait(Duration::from_secs(5), || {
            std::thread::sleep(Duration::from_millis(300));
            let now = server.status();
            let same = now.recent.back().map(|x| x.transaction) == s.recent.back().map(|x| x.transaction) && now.answered == s.answered;
            s = now;
            same
        }));
        // The answer whose write waits is counted and shown: its write has begun.
        let last = s.recent.back().unwrap().clone();
        let answered = s.answered.get(&0x03).copied().unwrap_or(0);
        assert_eq!(last.response.as_ref().map(|r| r.len()), Some(252), "{answered} answered");
        // A drop cuts it: taken back, its exchange showing none.
        server.drop_connections();
        assert!(wait(Duration::from_secs(5), || server.status().answered.get(&0x03).copied().unwrap_or(0) == answered - 1));
        let after = server.status();
        assert_eq!(after.connections, 0);
        assert_eq!(after.recent.back().unwrap().transaction, last.transaction);
        assert_eq!(after.recent.back().unwrap().response, None, "the answer a drop cut shows none");
    }

    /// G2 (wording). "withhold every answer not yet sent": silence begun while an answer waits on a
    /// client that reads nothing; the client then reads, and that answer arrives after silence began.
    #[test]
    fn an_answer_already_handed_to_its_socket_goes_out_after_silence_begins() {
        let server = start(&config("port = 0\nholding_registers = 125\n")).unwrap();
        let mut c = Raw::connect(server.addr);
        fill(&mut c);
        let mut s = server.status();
        assert!(wait(Duration::from_secs(5), || {
            std::thread::sleep(Duration::from_millis(300));
            let now = server.status();
            let same = now.recent.back().map(|x| x.transaction) == s.recent.back().map(|x| x.transaction) && now.answered == s.answered;
            s = now;
            same
        }));
        let waiting = s.recent.back().unwrap().transaction;
        server.set_silent(true);
        // Read every answer that comes.
        c.stream.set_nonblocking(false).unwrap();
        c.stream.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let mut got: Vec<u16> = Vec::new();
        loop {
            let mut h = [0u8; 7];
            if c.stream.read_exact(&mut h).is_err() {
                break;
            }
            let length = u16::from_be_bytes([h[4], h[5]]);
            let mut pdu = vec![0u8; length as usize - 1];
            if c.stream.read_exact(&mut pdu).is_err() {
                break;
            }
            got.push(u16::from_be_bytes([h[0], h[1]]));
        }
        // What was already handed to the socket goes out; nothing read after it is answered.
        assert_eq!(got.last(), Some(&waiting), "{} answers read after silence began", got.len());
        assert!(wait(Duration::from_secs(5), || server.status().unanswered > 0));
    }

    /// G3. "a length, quantity, byte count or coil value the function does not take with 03 before an
    /// address past its table with 02", for 0F, 10, 16 and 17 (modbus_server.rs pins it for 01, 04 and
    /// 05 only).
    #[test]
    fn the_03_comes_before_the_02_for_every_write_function() {
        let server = start(&config("port = 0\ncoils = 16\nholding_registers = 16\n")).unwrap();
        let mut c = Raw::connect(server.addr);
        let coils = |at: u16, n: u16, count: u8, bytes: usize| {
            let mut pdu = vec![0x0F];
            pdu.extend_from_slice(&at.to_be_bytes());
            pdu.extend_from_slice(&n.to_be_bytes());
            pdu.push(count);
            pdu.extend(std::iter::repeat_n(0xAA, bytes));
            pdu
        };
        let regs = |at: u16, n: u16, count: u8, bytes: usize| {
            let mut pdu = vec![0x10];
            pdu.extend_from_slice(&at.to_be_bytes());
            pdu.extend_from_slice(&n.to_be_bytes());
            pdu.push(count);
            pdu.extend(std::iter::repeat_n(0x01, bytes));
            pdu
        };
        let rw = |read_at: u16, read_n: u16, write_at: u16, write_n: u16, count: u8, bytes: usize| {
            let mut pdu = vec![0x17];
            for v in [read_at, read_n, write_at, write_n] {
                pdu.extend_from_slice(&v.to_be_bytes());
            }
            pdu.push(count);
            pdu.extend(std::iter::repeat_n(0x07, bytes));
            pdu
        };
        assert_eq!(c.ask(1, 1, &coils(0xFFF0, 1969, 247, 247)), [0x8F, 0x03], "0F: the quantity before the address");
        assert_eq!(c.ask(2, 1, &coils(0xFFF0, 9, 1, 1)), [0x8F, 0x03], "0F: the byte count before the address");
        assert_eq!(c.ask(3, 1, &regs(0xFFF0, 124, 248, 247)), [0x90, 0x03], "10: the quantity before the address");
        assert_eq!(c.ask(4, 1, &regs(0xFFF0, 2, 3, 3)), [0x90, 0x03], "10: the byte count before the address");
        assert_eq!(c.ask(5, 1, &[0x16, 0xFF, 0xF0, 0x00, 0x00, 0x00]), [0x96, 0x03], "16: the length before the address");
        assert_eq!(c.ask(6, 1, &rw(0xFFF0, 126, 0, 1, 2, 2)), [0x97, 0x03], "17: the read quantity before the read address");
        assert_eq!(c.ask(7, 1, &rw(0, 1, 0xFFF0, 122, 244, 243)), [0x97, 0x03], "17: the write quantity before the write address");
        assert_eq!(c.ask(8, 1, &rw(0, 1, 0xFFF0, 1, 3, 3)), [0x97, 0x03], "17: the byte count before the write address");
    }

    /// G4. "a length ... the function does not take with 03": 01, 02 and 06 one byte long, 10 and 17
    /// short of their fixed part, 17 a byte past its count (modbus_server.rs pins the length for 03, 05,
    /// 0F and 16 only).
    #[test]
    fn a_length_the_function_does_not_take_is_03_for_every_function() {
        let server = start(&config("port = 0\ncoils = 8\ndiscrete_inputs = 8\nholding_registers = 8\n")).unwrap();
        let mut c = Raw::connect(server.addr);
        server.set_registers(Table::HoldingRegister, 0, &[0xBEEF; 8]).unwrap();
        assert_eq!(c.ask(1, 1, &[0x01, 0x00, 0x00, 0x00, 0x01, 0x00]), [0x81, 0x03], "01, a byte long");
        assert_eq!(c.ask(2, 1, &[0x02, 0x00, 0x00, 0x00, 0x01, 0x00]), [0x82, 0x03], "02, a byte long");
        assert_eq!(c.ask(3, 1, &[0x06, 0x00, 0x00, 0x00, 0x01, 0x00]), [0x86, 0x03], "06, a byte long");
        assert_eq!(c.ask(4, 1, &[0x06, 0x00, 0x00, 0x00]), [0x86, 0x03], "06, a byte short");
        assert_eq!(c.ask(5, 1, &[0x10, 0x00, 0x00, 0x00, 0x01]), [0x90, 0x03], "10 without its byte count");
        assert_eq!(c.ask(6, 1, &[0x17, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01]), [0x97, 0x03], "17 without its byte count");
        assert_eq!(
            c.ask(7, 1, &[0x17, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x02, 0x00, 0x07, 0x00]),
            [0x97, 0x03],
            "17, a byte past its count"
        );
        assert_eq!(server.registers(Table::HoldingRegister, 0, 8).unwrap(), [0xBEEF; 8], "no refused request wrote");
    }

    /// G5. "(a refused request writes nothing)" for 05's 03: modbus_server.rs sends its bad coil values
    /// before it sets the coils apart, and overwrites them before it reads them.
    #[test]
    fn a_refused_single_coil_writes_nothing() {
        let server = start(&config("port = 0\ncoils = 2\n")).unwrap();
        let mut c = Raw::connect(server.addr);
        server.set_bits(Table::Coil, 0, &[false, true]).unwrap();
        assert_eq!(c.ask(1, 1, &[0x05, 0x00, 0x00, 0x12, 0x34]), [0x85, 0x03]);
        assert_eq!(c.ask(2, 1, &[0x05, 0x00, 0x01, 0x00, 0xFF]), [0x85, 0x03]);
        assert_eq!(server.bits(Table::Coil, 0, 2).unwrap(), [false, true], "a refused coil value wrote");
    }

    /// G6. "its knobs hold every answer for a delay": an exception is an answer.
    #[test]
    fn the_delay_holds_an_exception_too() {
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        let mut c = Raw::connect(server.addr);
        server.set_delay(Duration::from_millis(300));
        let asked = Instant::now();
        assert_eq!(c.ask(1, 1, &[0x07]), [0x87, 0x01]);
        assert!(asked.elapsed() >= Duration::from_millis(300), "{:?}", asked.elapsed());
        let asked = Instant::now();
        assert_eq!(c.ask(2, 1, &[0x03, 0x00, 0x01, 0x00, 0x01]), [0x83, 0x02]);
        assert!(asked.elapsed() >= Duration::from_millis(300), "{:?}", asked.elapsed());
    }

    /// G7. "a ... quantity ... the function does not take with 03": no register written by 10, none
    /// read or written by 17 (modbus_server.rs pins the zero quantity for 01, 03 and 0F only).
    #[test]
    fn a_quantity_of_zero_is_03_for_10_and_17() {
        let server = start(&config("port = 0\nholding_registers = 8\n")).unwrap();
        let mut c = Raw::connect(server.addr);
        assert_eq!(c.ask(1, 1, &[0x10, 0x00, 0x00, 0x00, 0x00, 0x00]), [0x90, 0x03], "10, no register");
        assert_eq!(c.ask(2, 1, &[0x17, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x00, 0x07]), [0x97, 0x03], "17, none read");
        assert_eq!(c.ask(3, 1, &[0x17, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00]), [0x97, 0x03], "17, none written");
        assert_eq!(server.registers(Table::HoldingRegister, 0, 1).unwrap(), [0], "nothing written");
    }

    /// G8. "each showing its answer once sent": two connections whose answers wait out a delay
    /// interleave; each exchange shows its own answer (a late answer filling the newest exchange
    /// instead passes modbus_server.rs, whose delayed answers never interleave).
    #[test]
    fn each_exchange_shows_its_own_answer_when_two_delayed_ones_interleave() {
        let server = start(&config("port = 0\nholding_registers = 2\n")).unwrap();
        server.set_registers(Table::HoldingRegister, 0, &[0x1111, 0x2222]).unwrap();
        server.set_delay(Duration::from_millis(300));
        let mut a = Raw::connect(server.addr);
        let mut b = Raw::connect(server.addr);
        a.send(&Raw::frame(1, 0, 1, &[0x03, 0x00, 0x00, 0x00, 0x01]));
        assert!(wait(Duration::from_secs(5), || server.status().recent.len() == 1));
        b.send(&Raw::frame(2, 0, 1, &[0x03, 0x00, 0x01, 0x00, 0x01]));
        assert!(wait(Duration::from_secs(5), || server.status().recent.len() == 2));
        assert_eq!(a.read().4, [0x03, 0x02, 0x11, 0x11]);
        assert_eq!(b.read().4, [0x03, 0x02, 0x22, 0x22]);
        let s = server.status();
        let shown: Vec<(u16, Option<Vec<u8>>)> = s.recent.iter().map(|x| (x.transaction, x.response.clone())).collect();
        assert_eq!(shown, [(1, Some(vec![0x03, 0x02, 0x11, 0x11])), (2, Some(vec![0x03, 0x02, 0x22, 0x22]))]);
    }
}

/// C-110's fourth refute round: the critic's probes, kept as pins (a client's own close during the
/// delay, a reset, an exception, the place under the limit freed at once; silence set under the
/// lock an answer's count takes).
mod round_4 {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use veredas_maquinar::modbus::*;

    fn config(text: &str) -> ModbusConfig {
        toml::from_str(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
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

    struct Raw {
        stream: TcpStream,
    }

    impl Raw {
        fn connect(addr: SocketAddr) -> Raw {
            let stream = TcpStream::connect(addr).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            stream.set_nodelay(true).unwrap();
            Raw { stream }
        }
        fn frame(transaction: u16, protocol: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
            let mut f = Vec::new();
            f.extend_from_slice(&transaction.to_be_bytes());
            f.extend_from_slice(&protocol.to_be_bytes());
            f.extend_from_slice(&(pdu.len() as u16 + 1).to_be_bytes());
            f.push(unit);
            f.extend_from_slice(pdu);
            f
        }
        fn send(&mut self, bytes: &[u8]) {
            self.stream.write_all(bytes).unwrap();
        }
    }

    fn total(m: &std::collections::BTreeMap<u8, u64>) -> u64 {
        m.values().sum()
    }

    /// P1. A client that closes its connection the ordinary way (FIN, what a client does when its
    /// timeout runs out) while its answer waits out a delay. The answer is never received; the
    /// clause's "taken back when the write fails or a drop cuts it (... one whose connection was
    /// dropped during its delay, counted as none)". The repository pins only the reset (linger 0).
    #[test]
    fn an_answer_to_a_client_that_closed_during_its_delay_is_counted_as_none() {
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        server.set_delay(Duration::from_millis(500));
        let mut c = Raw::connect(server.addr);
        c.send(&Raw::frame(1, 0, 1, &[0x03, 0x00, 0x00, 0x00, 0x01]));
        assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 1)));
        let closed = Instant::now();
        drop(c); // an ordinary close: FIN, no reset
        // The server notices only once the delay is over and its answer is written.
        let mut open_for = None;
        assert!(wait(Duration::from_secs(5), || {
            let gone = server.status().connections == 0;
            if gone && open_for.is_none() {
                open_for = Some(closed.elapsed());
            }
            gone
        }));
        let s = server.status();
        // Gone at once, well inside the 500 ms delay, nothing counted.
        assert!(open_for.unwrap() < Duration::from_millis(300), "the connection held {open_for:?} after the client closed");
        assert_eq!(s.answered.get(&0x03), None, "an answer written after its client closed is counted as answered");
        assert_eq!(s.recent.back().unwrap().response, None, "its exchange shows an answer no client received");
    }

    /// P1b. The same with a reset (the repository's G1a): taken back. Shown beside P1 so the
    /// difference is only how the client closed.
    #[test]
    fn the_same_client_closing_with_a_reset_counts_none_either() {
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        server.set_delay(Duration::from_millis(500));
        let mut c = Raw::connect(server.addr);
        c.send(&Raw::frame(1, 0, 1, &[0x03, 0x00, 0x00, 0x00, 0x01]));
        assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 1)));
        socket2::SockRef::from(&c.stream).set_linger(Some(Duration::ZERO)).unwrap();
        let closed = Instant::now();
        drop(c);
        assert!(wait(Duration::from_secs(5), || server.status().connections == 0));
        // Gone at once as well (refute round 5: a mutant ignoring the reset held it to the delay's end).
        assert!(closed.elapsed() < Duration::from_millis(300), "held {:?} after the reset", closed.elapsed());
        let s = server.status();
        eprintln!("P1b: answered = {:?}; exchange response = {:?}", s.answered, s.recent.back().unwrap().response);
        assert_eq!(s.answered.get(&0x03), None);
    }

    /// P3. An exception whose write fails is taken back too ("an answer counted ... taken back when
    /// the write fails"): the repository's G1a and G1b use a normal answer only.
    #[test]
    fn an_exception_whose_client_left_is_counted_as_none() {
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        server.set_delay(Duration::from_millis(300));
        let mut c = Raw::connect(server.addr);
        c.send(&Raw::frame(1, 0, 1, &[0x07]));
        assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 1)));
        socket2::SockRef::from(&c.stream).set_linger(Some(Duration::ZERO)).unwrap();
        drop(c);
        assert!(wait(Duration::from_secs(5), || server.status().connections == 0));
        let s = server.status();
        eprintln!("P3: exceptions = {:?}; exchange response = {:?}", s.exceptions, s.recent.back().unwrap().response);
        assert!(s.exceptions.is_empty(), "an exception never sent stays counted: {:?}", s.exceptions);
        assert_eq!(s.recent.back().unwrap().response, None);
    }

    /// P2. "withhold every answer not yet handed to its socket": a client pipelining and reading at
    /// once, no delay; silence switched on while requests are in flight. Any answer counted (its write
    /// begun) after set_silent(true) returned and a status was taken was not handed to its socket when
    /// silence began, and goes out.
    fn silence_window(hammer: bool) -> (u32, u32, u64) {
        let server = Arc::new(start(&config("port = 0\nholding_registers = 4\n")).unwrap());
        let stream = TcpStream::connect(server.addr).unwrap();
        stream.set_nodelay(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let received = Arc::new(AtomicU64::new(0));
        let mut w = stream.try_clone().unwrap();
        let stop_w = Arc::clone(&stop);
        let writer = std::thread::spawn(move || {
            let burst = Raw::frame(1, 0, 1, &[0x03, 0x00, 0x00, 0x00, 0x01]).repeat(8);
            while !stop_w.load(Ordering::Relaxed) {
                if w.write_all(&burst).is_err() {
                    break;
                }
            }
        });
        let mut r = stream.try_clone().unwrap();
        r.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        let stop_r = Arc::clone(&stop);
        let rec = Arc::clone(&received);
        let reader = std::thread::spawn(move || {
            let mut chunk = vec![0u8; 1 << 16];
            while !stop_r.load(Ordering::Relaxed) {
                match r.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        rec.fetch_add(n as u64, Ordering::Relaxed);
                    }
                    Err(_) => {}
                }
            }
        });
        let hammering = if hammer {
            let srv = Arc::clone(&server);
            let stop_h = Arc::clone(&stop);
            Some(std::thread::spawn(move || {
                while !stop_h.load(Ordering::Relaxed) {
                    let _ = srv.status();
                }
            }))
        } else {
            None
        };
        let mut violations = 0u32;
        let mut excess = 0u64;
        let trials = 100u32;
        for i in 0..trials {
            std::thread::sleep(Duration::from_millis(3 + (i % 7) as u64));
            server.set_silent(true);
            let before = total(&server.status().answered);
            std::thread::sleep(Duration::from_millis(30));
            let after = total(&server.status().answered);
            if after > before {
                violations += 1;
                excess += after - before;
            }
            server.set_silent(false);
        }
        stop.store(true, Ordering::Relaxed);
        let _ = stream.shutdown(std::net::Shutdown::Both);
        let _ = writer.join();
        let _ = reader.join();
        if let Some(h) = hammering {
            let _ = h.join();
        }
        (violations, trials, excess)
    }

    #[test]
    fn no_answer_is_handed_to_its_socket_once_silence_is_set_whatever_else_holds_the_lock() {
        let (v, n, excess) = silence_window(true);
        eprintln!("P2b (a status reader on another thread): {v} of {n} trials counted (and sent) {excess} answer(s) whose write began after set_silent(true) returned");
        assert_eq!(v, 0);
    }

    /// P1c. The consequence for the limit: a client that closed (FIN) during a delay still holds its
    /// place until the delay is over, so the one connection a limit of one allows is refused.
    #[test]
    fn a_client_that_closed_during_a_delay_frees_its_place_under_the_limit() {
        let server = start(&config("port = 0\nmax_connections = 1\nholding_registers = 1\n")).unwrap();
        server.set_delay(Duration::from_secs(3));
        let mut a = Raw::connect(server.addr);
        a.send(&Raw::frame(1, 0, 1, &[0x03, 0x00, 0x00, 0x00, 0x01]));
        assert!(wait(Duration::from_secs(5), || server.status().recent.len() == 1));
        drop(a);
        std::thread::sleep(Duration::from_millis(100));
        server.set_delay(Duration::ZERO);
        let mut b = Raw::connect(server.addr);
        b.send(&Raw::frame(2, 0, 1, &[0x03, 0x00, 0x00, 0x00, 0x01]));
        let mut h = [0u8; 1];
        let got = b.stream.read(&mut h);
        let s = server.status();
        eprintln!("P1c: second client's read = {got:?}; connections = {}, refused = {}", s.connections, s.refused);
        assert_eq!(s.refused, 0, "the only open connection's client had closed");
    }
}

/// C-110's fifth refute round: the critic's probes (`~/.cache/veredas/critic-c110-r5/probes`), kept
/// as pins: a close or a reset seen during the delay whatever was pipelined behind, a half-close
/// taken as gone, a place freed for a client connecting again at once, a silenced answer counted
/// unanswered however its delay ends, a failed write's take-back.
mod round_5 {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::time::{Duration, Instant};

    use veredas_maquinar::modbus::*;

    fn config(text: &str) -> ModbusConfig {
        toml::from_str(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
    }

    fn wait(within: Duration, mut what: impl FnMut() -> bool) -> bool {
        let started = Instant::now();
        while started.elapsed() < within {
            if what() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        what()
    }

    fn connect(addr: SocketAddr) -> TcpStream {
        let s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.set_nodelay(true).unwrap();
        s
    }

    fn frame(transaction: u16, pdu: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&transaction.to_be_bytes());
        f.extend_from_slice(&0u16.to_be_bytes());
        f.extend_from_slice(&(pdu.len() as u16 + 1).to_be_bytes());
        f.push(1);
        f.extend_from_slice(pdu);
        f
    }

    const READ: [u8; 5] = [0x03, 0x00, 0x00, 0x00, 0x01];

    fn answered(s: &ModbusStatus) -> u64 {
        s.answered.values().sum()
    }

    /// How long the server kept the connection after `closed`, and the status once it was gone.
    fn gone_after(server: &ModbusHandle, closed: Instant) -> (Duration, ModbusStatus) {
        let mut open_for = None;
        assert!(wait(Duration::from_secs(10), || {
            let gone = server.status().connections == 0;
            if gone && open_for.is_none() {
                open_for = Some(closed.elapsed());
            }
            gone
        }));
        (open_for.unwrap(), server.status())
    }

    #[test]
    fn a_client_that_closes_or_resets_during_the_delay_is_gone_at_once_whatever_it_pipelined() {
        // (a second request pipelined behind the delayed one, a reset rather than a FIN)
        for (pipelined, reset) in [(true, false), (true, true), (false, true)] {
            let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
            server.set_delay(Duration::from_millis(800));
            let mut c = connect(server.addr);
            let sent = if pipelined { [frame(1, &READ), frame(2, &READ)].concat() } else { frame(1, &READ) };
            c.write_all(&sent).unwrap();
            assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 1)));
            std::thread::sleep(Duration::from_millis(50));
            if reset {
                socket2::SockRef::from(&c).set_linger(Some(Duration::ZERO)).unwrap();
            }
            let closed = Instant::now();
            drop(c);
            let (open_for, s) = gone_after(&server, closed);
            let case = format!("pipelined {pipelined}, reset {reset}");
            assert!(open_for < Duration::from_millis(300), "{case}: held {open_for:?} after the client left");
            assert_eq!(answered(&s), 0, "{case}: {:?}", s.answered);
            assert!(s.recent.iter().all(|x| x.response.is_none()), "{case}: {:?}", s.recent);
        }
    }

    #[test]
    fn a_client_that_pipelined_and_closed_during_a_delay_frees_its_place_under_a_limit_of_one() {
        let server = start(&config("port = 0\nmax_connections = 1\nholding_registers = 1\n")).unwrap();
        server.set_delay(Duration::from_secs(3));
        let mut a = connect(server.addr);
        a.write_all(&[frame(1, &READ), frame(2, &READ)].concat()).unwrap();
        assert!(wait(Duration::from_secs(5), || server.status().recent.len() == 1));
        drop(a);
        assert!(wait(Duration::from_secs(2), || server.status().connections == 0), "the place still held");
        server.set_delay(Duration::ZERO);
        let mut b = connect(server.addr);
        b.write_all(&frame(3, &READ)).unwrap();
        let mut h = [0u8; 11];
        b.read_exact(&mut h).unwrap();
        assert_eq!((&h[..2], server.status().refused), (&[0u8, 3][..], 0));
    }

    #[test]
    fn a_half_closing_client_is_answered_without_a_delay_and_taken_as_gone_with_one() {
        // A client that sends and shuts its sending side cannot be told from one that closed: under
        // a delay its FIN is read as the client gone, and its answer never sent.
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        let ask = |delay_ms: u64| -> Vec<u8> {
            server.set_delay(Duration::from_millis(delay_ms));
            let mut c = connect(server.addr);
            c.write_all(&frame(7, &READ)).unwrap();
            c.shutdown(std::net::Shutdown::Write).unwrap();
            let mut got = Vec::new();
            c.read_to_end(&mut got).unwrap();
            got
        };
        assert_eq!(ask(0).len(), 11);
        assert_eq!(ask(300), Vec::<u8>::new());
        let s = server.status();
        assert_eq!((answered(&s), s.unanswered), (1, 0));
    }

    #[test]
    fn an_answer_silence_withheld_is_counted_unanswered_whether_a_drop_or_the_client_ends_its_delay() {
        for how in ["drop", "close"] {
            let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
            server.set_delay(Duration::from_millis(800));
            let mut c = connect(server.addr);
            c.write_all(&frame(1, &READ)).unwrap();
            assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 1)));
            server.set_silent(true);
            std::thread::sleep(Duration::from_millis(50));
            if how == "drop" {
                server.drop_connections();
            } else {
                drop(c);
            }
            assert!(wait(Duration::from_secs(5), || server.status().connections == 0));
            let s = server.status();
            assert_eq!((answered(&s), s.unanswered), (0, 1), "{how}");
        }
        // Without silence, a client gone during the delay counts as neither.
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        server.set_delay(Duration::from_millis(800));
        let mut c = connect(server.addr);
        c.write_all(&frame(1, &READ)).unwrap();
        assert!(wait(Duration::from_secs(5), || server.status().recent.len() == 1));
        drop(c);
        assert!(wait(Duration::from_secs(5), || server.status().connections == 0));
        let s = server.status();
        assert_eq!((answered(&s), s.unanswered), (0, 0));
    }

    #[test]
    fn an_answer_whose_waiting_write_fails_on_the_client_s_reset_is_taken_back() {
        // The client sends without reading till the server's write waits on it, then resets: the
        // write fails (no drop), and the answer it carried is taken back.
        let server = start(&config("port = 0\nholding_registers = 125\n")).unwrap();
        let mut c = connect(server.addr);
        c.set_nonblocking(true).unwrap();
        let mut next: u16 = 0;
        let mut pending: Vec<u8> = Vec::new();
        let started = Instant::now();
        let mut blocked = 0;
        while blocked < 200 && started.elapsed() < Duration::from_secs(60) {
            if pending.is_empty() {
                for _ in 0..100 {
                    pending.extend(frame(next, &[0x03, 0x00, 0x00, 0x00, 0x7D]));
                    next = next.wrapping_add(1);
                }
            }
            match c.write(&pending) {
                Ok(n) => {
                    pending.drain(..n);
                    blocked = 0;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    blocked += 1;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("{e}"),
            }
        }
        assert_eq!(blocked, 200, "the server stopped reading");
        // Settled: the last exchange and the counts unchanged across 300 ms.
        let mut s = server.status();
        assert!(wait(Duration::from_secs(10), || {
            std::thread::sleep(Duration::from_millis(300));
            let now = server.status();
            let same = now.recent.back().map(|x| x.transaction) == s.recent.back().map(|x| x.transaction) && now.answered == s.answered;
            s = now;
            same
        }));
        let before = answered(&s);
        assert!(s.recent.back().unwrap().response.is_some(), "the waiting write counted and shown");
        socket2::SockRef::from(&c).set_linger(Some(Duration::ZERO)).unwrap();
        drop(c);
        assert!(wait(Duration::from_secs(5), || server.status().connections == 0));
        let after = server.status();
        assert_eq!(answered(&after), before - 1, "the answer whose write failed taken back");
        assert_eq!(after.recent.back().unwrap().response, None);
    }

    #[test]
    fn a_client_closing_and_connecting_again_at_once_under_a_limit_of_one_is_never_refused() {
        let server = start(&config("port = 0\nmax_connections = 1\nholding_registers = 1\n")).unwrap();
        // Between answers: 500 times.
        let mut c = connect(server.addr);
        for i in 0..500u16 {
            c.write_all(&frame(i, &READ)).unwrap();
            let mut h = [0u8; 11];
            c.read_exact(&mut h).unwrap_or_else(|e| panic!("trial {i}: refused ({e})"));
            drop(c);
            c = connect(server.addr);
        }
        drop(c);
        // During a delay: 300 times.
        for i in 0..300u16 {
            assert!(wait(Duration::from_secs(5), || server.status().connections == 0));
            server.set_delay(Duration::from_secs(3));
            let mut a = connect(server.addr);
            a.write_all(&frame(i, &READ)).unwrap();
            let me = a.local_addr().unwrap();
            assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == i && x.peer == me)));
            server.set_delay(Duration::ZERO);
            drop(a);
            let mut b = connect(server.addr);
            b.write_all(&frame(i, &READ)).unwrap();
            let mut h = [0u8; 11];
            b.read_exact(&mut h).unwrap_or_else(|e| panic!("delayed trial {i}: refused ({e})"));
        }
        assert_eq!(server.status().refused, 0);
    }
}

/// C-110's sixth refute round: the critic's probes (`~/.cache/veredas/critic-c110-r6/probes`), kept
/// as pins: delays side by side, a close behind many requests, requests read ahead in order across
/// the cap, a request read ahead during a delay silence touched, a reconnection beside busy
/// connections, several at once and drops racing them, silence that came and went, frames read
/// ahead judged in turn, past the cap, a pipelined write once its client is gone.
mod round_6 {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use veredas_maquinar::modbus::*;

    fn config(text: &str) -> ModbusConfig {
        toml::from_str(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
    }

    fn wait(within: Duration, mut what: impl FnMut() -> bool) -> bool {
        let started = Instant::now();
        while started.elapsed() < within {
            if what() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        what()
    }

    fn connect(addr: SocketAddr) -> TcpStream {
        let s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.set_nodelay(true).unwrap();
        s
    }

    fn frame_p(transaction: u16, protocol: u16, pdu: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&transaction.to_be_bytes());
        f.extend_from_slice(&protocol.to_be_bytes());
        f.extend_from_slice(&(pdu.len() as u16 + 1).to_be_bytes());
        f.push(1);
        f.extend_from_slice(pdu);
        f
    }

    fn frame(transaction: u16, pdu: &[u8]) -> Vec<u8> {
        frame_p(transaction, 0, pdu)
    }

    const READ: [u8; 5] = [0x03, 0x00, 0x00, 0x00, 0x01];

    fn read_answer(s: &mut TcpStream) -> std::io::Result<(u16, Vec<u8>)> {
        let mut h = [0u8; 7];
        s.read_exact(&mut h)?;
        let length = u16::from_be_bytes([h[4], h[5]]);
        let mut pdu = vec![0u8; length as usize - 1];
        s.read_exact(&mut pdu)?;
        Ok((u16::from_be_bytes([h[0], h[1]]), pdu))
    }

    /// Every answer that arrives within `d`, by transaction.
    fn answers_within(s: &mut TcpStream, d: Duration) -> Vec<u16> {
        s.set_read_timeout(Some(d)).unwrap();
        let mut got = Vec::new();
        while let Ok((t, _)) = read_answer(s) {
            got.push(t);
        }
        got
    }

    fn answered(s: &ModbusStatus) -> u64 {
        s.answered.values().sum()
    }

    #[test]
    fn two_connections_wait_out_their_delays_together() {
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        server.set_delay(Duration::from_millis(500));
        let mut a = connect(server.addr);
        let mut b = connect(server.addr);
        assert!(wait(Duration::from_secs(5), || server.status().connections == 2));
        let t0 = Instant::now();
        a.write_all(&frame(1, &READ)).unwrap();
        b.write_all(&frame(2, &READ)).unwrap();
        read_answer(&mut a).unwrap();
        let ta = t0.elapsed();
        read_answer(&mut b).unwrap();
        let tb = t0.elapsed();
        assert!(ta >= Duration::from_millis(500) && tb >= Duration::from_millis(500));
        assert!(tb < Duration::from_millis(900), "the second connection waited out the first's delay too: {tb:?}");
    }

    #[test]
    fn a_close_behind_many_pipelined_requests_is_seen_at_once() {
        for (n, reset) in [(2usize, false), (100, false), (5000, false), (5000, true)] {
            let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
            server.set_delay(Duration::from_millis(800));
            let mut c = connect(server.addr);
            c.write_all(&frame(0, &READ)).unwrap();
            assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 0)));
            let behind: Vec<u8> = (1..=n as u16).flat_map(|t| frame(t, &READ)).collect();
            c.write_all(&behind).unwrap();
            std::thread::sleep(Duration::from_millis(50));
            if reset {
                socket2::SockRef::from(&c).set_linger(Some(Duration::ZERO)).unwrap();
            }
            let closed = Instant::now();
            drop(c);
            assert!(wait(Duration::from_secs(10), || server.status().connections == 0));
            let held = closed.elapsed();
            assert!(held < Duration::from_millis(300), "{n} behind, reset {reset}: held {held:?}");
            assert_eq!(answered(&server.status()), 0, "{n} behind");
        }
    }

    #[test]
    fn requests_read_ahead_are_served_in_their_order_across_the_cap() {
        // 6000 requests (72 000 bytes) behind a delayed one, past the 64 KiB read ahead: one lies
        // half read ahead, half in the socket; every one answered once, in order.
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        server.set_delay(Duration::from_millis(400));
        let mut c = connect(server.addr);
        c.write_all(&frame(0, &READ)).unwrap();
        assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 0)));
        let n = 6000u16;
        let behind: Vec<u8> = (1..=n).flat_map(|t| frame(t, &READ)).collect();
        c.write_all(&behind).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        server.set_delay(Duration::ZERO);
        let got = answers_within(&mut c, Duration::from_secs(3));
        assert_eq!(got, (0..=n).collect::<Vec<u16>>());
    }

    #[test]
    fn a_request_read_ahead_during_a_delay_silence_touched_is_withheld_with_it() {
        // Read (ahead) while silent, silence ending before the delay does: never answered, its
        // write never done.
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        server.set_delay(Duration::from_millis(600));
        let mut c = connect(server.addr);
        c.write_all(&frame(1, &READ)).unwrap();
        assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 1)));
        server.set_silent(true);
        std::thread::sleep(Duration::from_millis(30));
        c.write_all(&frame(2, &[0x06, 0x00, 0x00, 0x00, 0x55])).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        server.set_silent(false);
        assert!(answers_within(&mut c, Duration::from_millis(2500)).is_empty());
        let s = server.status();
        assert_eq!((server.registers(Table::HoldingRegister, 0, 1).unwrap(), answered(&s), s.unanswered), (vec![0], 0, 2));
        // Pipelined before silence came and went: its answer not handed to its socket then.
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        server.set_delay(Duration::from_millis(600));
        let mut c = connect(server.addr);
        c.write_all(&[frame(1, &READ), frame(2, &READ)].concat()).unwrap();
        assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 1)));
        std::thread::sleep(Duration::from_millis(50));
        server.set_silent(true);
        std::thread::sleep(Duration::from_millis(30));
        server.set_silent(false);
        assert!(answers_within(&mut c, Duration::from_millis(2500)).is_empty());
        assert_eq!(server.status().unanswered, 2);
        // Without a delay, as before: read while silent, never answered.
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        let mut c = connect(server.addr);
        server.set_silent(true);
        c.write_all(&frame(2, &READ)).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        server.set_silent(false);
        assert!(answers_within(&mut c, Duration::from_millis(1500)).is_empty());
        assert_eq!(server.status().unanswered, 1);
        // And a request read after the delay ends is answered: silence touched only what came before.
        c.write_all(&frame(3, &READ)).unwrap();
        assert_eq!(read_answer(&mut c).unwrap().0, 3);
    }

    #[test]
    fn a_client_reconnecting_at_once_at_the_limit_beside_99_busy_connections_is_not_refused() {
        let n = 100usize;
        let server = start(&config(&format!("port = 0\nmax_connections = {n}\nholding_registers = 1\n"))).unwrap();
        let addr = server.addr;
        let stop = Arc::new(AtomicBool::new(false));
        let busy: Vec<_> = (0..n - 1)
            .map(|i| {
                let mut s = connect(addr);
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    let mut t = 0u16;
                    while !stop.load(Ordering::Relaxed) {
                        t = t.wrapping_add(1);
                        assert!(s.write_all(&frame(t, &READ)).is_ok() && read_answer(&mut s).is_ok(), "busy client {i} lost its connection");
                    }
                })
            })
            .collect();
        assert!(wait(Duration::from_secs(10), || server.status().connections == n - 1));
        let mut c = connect(addr);
        let mut failed = 0;
        for i in 0..300u16 {
            if !(c.write_all(&frame(i, &READ)).is_ok() && read_answer(&mut c).is_ok()) {
                failed += 1;
            }
            drop(c);
            c = connect(addr);
        }
        drop(c);
        stop.store(true, Ordering::Relaxed);
        for b in busy {
            b.join().unwrap();
        }
        assert_eq!((failed, server.status().refused), (0, 0));
    }

    #[test]
    fn several_clients_reconnecting_at_once_at_the_limit_and_drops_racing_them_leave_the_counts_whole() {
        let server = start(&config("port = 0\nmax_connections = 4\nholding_registers = 1\n")).unwrap();
        let addr = server.addr;
        let threads: Vec<_> = (0..4)
            .map(|_| {
                std::thread::spawn(move || {
                    let mut failed = 0;
                    let mut c = connect(addr);
                    for i in 0..200u16 {
                        if !(c.write_all(&frame(i, &READ)).is_ok() && read_answer(&mut c).is_ok()) {
                            failed += 1;
                        }
                        drop(c);
                        c = connect(addr);
                    }
                    failed
                })
            })
            .collect();
        let failed: u32 = threads.into_iter().map(|t| t.join().unwrap()).sum();
        assert_eq!((failed, server.status().refused), (0, 0));
        // Drops racing reconnections at a limit of one: the counts end whole, a new client served.
        let server = Arc::new(start(&config("port = 0\nmax_connections = 1\nholding_registers = 1\n")).unwrap());
        let addr = server.addr;
        let done = Arc::new(AtomicBool::new(false));
        let client = {
            let done = Arc::clone(&done);
            std::thread::spawn(move || {
                for i in 0..400u16 {
                    let mut c = connect(addr);
                    let _ = c.write_all(&frame(i, &READ)).is_ok() && read_answer(&mut c).is_ok();
                }
                done.store(true, Ordering::Relaxed);
            })
        };
        while !done.load(Ordering::Relaxed) {
            server.drop_connections();
            std::thread::sleep(Duration::from_micros(500));
        }
        client.join().unwrap();
        assert!(wait(Duration::from_secs(5), || server.status().connections == 0));
        let mut fresh = connect(addr);
        fresh.write_all(&frame(9, &READ)).unwrap();
        assert_eq!(read_answer(&mut fresh).unwrap().0, 9);
    }

    #[test]
    fn silence_that_came_and_went_then_the_client_left_counts_the_answer_unanswered() {
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        server.set_delay(Duration::from_millis(800));
        let c = connect(server.addr);
        (&c).write_all(&frame(1, &READ)).unwrap();
        assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 1)));
        server.set_silent(true);
        std::thread::sleep(Duration::from_millis(30));
        server.set_silent(false);
        std::thread::sleep(Duration::from_millis(30));
        drop(c);
        assert!(wait(Duration::from_secs(5), || server.status().connections == 0));
        let s = server.status();
        assert_eq!((answered(&s), s.unanswered), (0, 1));
    }

    #[test]
    fn foreign_and_malformed_frames_read_ahead_are_judged_in_their_turn() {
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        server.set_delay(Duration::from_millis(300));
        let mut c = connect(server.addr);
        c.write_all(&frame(1, &READ)).unwrap();
        assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 1)));
        c.write_all(&[frame_p(2, 7, &[0x06, 0x00, 0x00, 0x00, 0x09]), frame(3, &READ)].concat()).unwrap();
        assert_eq!(answers_within(&mut c, Duration::from_millis(1500)), [1, 3]);
        assert_eq!((server.status().dropped, server.registers(Table::HoldingRegister, 0, 1).unwrap()), (1, vec![0]));
        let mut d = connect(server.addr);
        d.write_all(&frame(1, &READ)).unwrap();
        assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 1 && x.peer == d.local_addr().unwrap())));
        d.write_all(&[0, 9, 0, 0, 0, 0, 1, 3]).unwrap();
        d.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        assert_eq!(read_answer(&mut d).unwrap().0, 1);
        let mut b = [0u8; 1];
        assert!(matches!(d.read(&mut b), Ok(0) | Err(_)));
        assert_eq!(server.status().malformed, 1);
    }

    #[test]
    fn past_64_kib_read_ahead_a_close_is_seen_only_by_a_failing_write() {
        // 72 000 bytes behind an 800 ms delay, then the client closes: past the cap the close is
        // not seen at once; the answer is written and counted, the next request done.
        let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
        server.set_delay(Duration::from_millis(800));
        let mut c = connect(server.addr);
        c.write_all(&frame(0, &READ)).unwrap();
        assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == 0)));
        let behind: Vec<u8> = (1..=6000u16).flat_map(|t| frame(t, &READ)).collect();
        c.write_all(&behind).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let closed = Instant::now();
        drop(c);
        assert!(wait(Duration::from_secs(10), || server.status().connections == 0));
        assert!(closed.elapsed() >= Duration::from_millis(600), "held {:?}: the cap would make it a delay at least", closed.elapsed());
        assert!(answered(&server.status()) >= 1);
    }

    #[test]
    fn a_write_pipelined_behind_a_delayed_request_is_neither_done_nor_shown_once_its_client_is_gone() {
        for (delay_ms, half, lands) in [(0u64, true, true), (600, true, false), (600, false, false)] {
            let server = start(&config("port = 0\nholding_registers = 1\n")).unwrap();
            server.set_delay(Duration::from_millis(delay_ms));
            let c = connect(server.addr);
            (&c).write_all(&[frame(1, &READ), frame(2, &[0x06, 0x00, 0x00, 0x00, 0x77])].concat()).unwrap();
            std::thread::sleep(Duration::from_millis(50));
            if half {
                c.shutdown(std::net::Shutdown::Write).unwrap();
            }
            std::thread::sleep(Duration::from_millis(20));
            if half && delay_ms == 0 {
                let _ = (&c).read_to_end(&mut Vec::new());
            }
            drop(c);
            assert!(wait(Duration::from_secs(5), || server.status().connections == 0));
            let s = server.status();
            let shown: Vec<u16> = s.recent.iter().map(|x| x.transaction).collect();
            assert_eq!(server.registers(Table::HoldingRegister, 0, 1).unwrap(), [if lands { 0x77 } else { 0 }], "delay {delay_ms}, half {half}");
            assert_eq!(shown, if lands { vec![1, 2] } else { vec![1] }, "delay {delay_ms}, half {half}");
        }
    }
}

/// C-110's seventh refute round: the critic's probes (`~/.cache/veredas/critic-c110-r7/probes`),
/// kept as pins: the accept loop's yields at a limit of 300, a request after withheld ones answered,
/// one read ahead during an untouched delay not withheld by a later silence, the read-ahead's exact
/// cap and what comes past it, a connection past the limit closed within 300 ms.
mod round_7 {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use veredas_maquinar::modbus::*;

    fn config(text: &str) -> ModbusConfig {
        toml::from_str(text).unwrap_or_else(|e| panic!("{e}\n{text}"))
    }

    fn wait(within: Duration, mut what: impl FnMut() -> bool) -> bool {
        let started = Instant::now();
        while started.elapsed() < within {
            if what() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        what()
    }

    fn connect(addr: SocketAddr) -> TcpStream {
        let s = TcpStream::connect(addr).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        s.set_nodelay(true).unwrap();
        s
    }

    fn frame(transaction: u16, pdu: &[u8]) -> Vec<u8> {
        let mut f = Vec::new();
        f.extend_from_slice(&transaction.to_be_bytes());
        f.extend_from_slice(&0u16.to_be_bytes());
        f.extend_from_slice(&(pdu.len() as u16 + 1).to_be_bytes());
        f.push(1);
        f.extend_from_slice(pdu);
        f
    }

    const READ: [u8; 5] = [0x03, 0x00, 0x00, 0x00, 0x01];

    fn read_answer(s: &mut TcpStream) -> std::io::Result<(u16, Vec<u8>)> {
        let mut h = [0u8; 7];
        s.read_exact(&mut h)?;
        let length = u16::from_be_bytes([h[4], h[5]]);
        let mut pdu = vec![0u8; length as usize - 1];
        s.read_exact(&mut pdu)?;
        Ok((u16::from_be_bytes([h[0], h[1]]), pdu))
    }

    /// The transactions answered until `d` passes without one.
    fn answers_within(s: &mut TcpStream, d: Duration) -> Vec<u16> {
        s.set_read_timeout(Some(d)).unwrap();
        let mut got = Vec::new();
        while let Ok((t, _)) = read_answer(s) {
            got.push(t);
        }
        got
    }

    fn answered(s: &ModbusStatus) -> u64 {
        s.answered.values().sum()
    }

    /// Waits till the server shows transaction `t` as read.
    fn read_by_server(server: &ModbusHandle, t: u16) {
        assert!(wait(Duration::from_secs(5), || server.status().recent.back().is_some_and(|x| x.transaction == t)), "transaction {t} read");
    }

    #[test]
    fn a_client_reconnecting_at_once_beside_299_busy_connections_is_not_refused() {
        // 299 busy: three yields (183 tasks) are not enough, one per 61 connections is.
        let (n, threads) = (300usize, 16usize);
        let server = start(&config(&format!("port = 0\nmax_connections = {n}\nholding_registers = 1\n"))).unwrap();
        let addr = server.addr;
        let stop = Arc::new(AtomicBool::new(false));
        let lost = Arc::new(AtomicU64::new(0));
        let mut socks: Vec<Vec<TcpStream>> = (0..threads).map(|_| Vec::new()).collect();
        for i in 0..n - 1 {
            socks[i % threads].push(connect(addr));
        }
        assert!(wait(Duration::from_secs(20), || server.status().connections == n - 1));
        let busy: Vec<_> = socks
            .into_iter()
            .map(|mut mine| {
                let (stop, lost) = (Arc::clone(&stop), Arc::clone(&lost));
                std::thread::spawn(move || {
                    let mut t = 0u16;
                    while !stop.load(Ordering::Relaxed) {
                        t = t.wrapping_add(1);
                        for s in mine.iter_mut() {
                            if s.write_all(&frame(t, &READ)).is_err() {
                                lost.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        for s in mine.iter_mut() {
                            if read_answer(s).is_err() {
                                lost.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                })
            })
            .collect();
        std::thread::sleep(Duration::from_millis(300));
        let mut c = connect(addr);
        let mut failed = 0;
        for i in 0..300u16 {
            if !(c.write_all(&frame(i, &READ)).is_ok() && read_answer(&mut c).is_ok()) {
                failed += 1;
            }
            drop(c);
            c = connect(addr);
        }
        drop(c);
        stop.store(true, Ordering::Relaxed);
        for b in busy {
            b.join().unwrap();
        }
        assert_eq!((failed, server.status().refused, lost.load(Ordering::Relaxed)), (0, 0, 0));
    }

    #[test]
    fn requests_after_the_withheld_ones_are_answered_and_done() {
        let server = start(&config("port = 0\nholding_registers = 2\n")).unwrap();
        server.set_delay(Duration::from_millis(600));
        let mut c = connect(server.addr);
        c.write_all(&frame(1, &READ)).unwrap();
        read_by_server(&server, 1);
        server.set_silent(true);
        std::thread::sleep(Duration::from_millis(30));
        c.write_all(&frame(2, &[0x06, 0x00, 0x00, 0x00, 0x55])).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        server.set_silent(false);
        assert!(answers_within(&mut c, Duration::from_millis(1500)).is_empty());
        // Silence over, the delay over: a write and a read, answered in order after their delays.
        c.write_all(&[frame(3, &[0x06, 0x00, 0x01, 0x00, 0x66]), frame(4, &READ)].concat()).unwrap();
        assert_eq!(answers_within(&mut c, Duration::from_millis(3000)), [3, 4]);
        let s = server.status();
        assert_eq!((server.registers(Table::HoldingRegister, 0, 2).unwrap(), answered(&s), s.unanswered), (vec![0, 0x66], 2, 2));
    }

    #[test]
    fn a_request_read_ahead_during_a_delay_silence_did_not_touch_is_answered() {
        let server = start(&config("port = 0\nholding_registers = 2\n")).unwrap();
        server.set_delay(Duration::from_millis(600));
        let mut c = connect(server.addr);
        c.write_all(&frame(1, &READ)).unwrap();
        read_by_server(&server, 1);
        // Two requests read ahead during request 1's delay, no silence there.
        c.write_all(&[frame(2, &READ), frame(3, &[0x06, 0x00, 0x01, 0x00, 0x33])].concat()).unwrap();
        assert_eq!(read_answer(&mut c).unwrap().0, 1);
        // Silence comes and goes inside request 2's own delay: 2 withheld, 3 answered and done.
        read_by_server(&server, 2);
        std::thread::sleep(Duration::from_millis(50));
        server.set_silent(true);
        std::thread::sleep(Duration::from_millis(30));
        server.set_silent(false);
        assert_eq!(answers_within(&mut c, Duration::from_millis(2500)), [3]);
        let s = server.status();
        assert_eq!((server.registers(Table::HoldingRegister, 1, 1).unwrap(), answered(&s), s.unanswered), (vec![0x33], 2, 1));
    }

    /// On the machine's lock: the read-ahead fills during an 800 ms delay.
    #[test]
    fn the_read_ahead_stops_at_64_kib_and_a_request_past_it_is_judged_when_it_is_read() {
        let server = start(&config("port = 0\nholding_registers = 2\n")).unwrap();
        server.set_delay(Duration::from_millis(800));
        let mut c = connect(server.addr);
        c.write_all(&frame(0, &READ)).unwrap();
        read_by_server(&server, 0);
        server.set_silent(true);
        // 6001 requests of 12 bytes while silent, the last a write: the 5462 that begin within
        // 64 KiB are read ahead during the delay and withheld with it; the rest are read once the
        // delay is over and silence lifted, and answered.
        let mut behind: Vec<u8> = (1..=6000u16).flat_map(|t| frame(t, &READ)).collect();
        behind.extend(frame(6001, &[0x06, 0x00, 0x01, 0x00, 0x44]));
        c.write_all(&behind).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        server.set_silent(false);
        server.set_delay(Duration::ZERO);
        let got = answers_within(&mut c, Duration::from_secs(3));
        let s = server.status();
        assert_eq!(got, (5463..=6001).collect::<Vec<u16>>());
        assert_eq!((server.registers(Table::HoldingRegister, 1, 1).unwrap(), answered(&s), s.unanswered), (vec![0x44], 539, 5463));
    }

    /// On the machine's lock: a bound on the close.
    #[test]
    fn a_connection_past_the_limit_is_closed_within_300_ms() {
        let server = start(&config("port = 0\nmax_connections = 1\nholding_registers = 1\n")).unwrap();
        let mut a = connect(server.addr);
        a.write_all(&frame(1, &READ)).unwrap();
        assert_eq!(read_answer(&mut a).unwrap().0, 1);
        for _ in 0..5 {
            let mut extra = connect(server.addr);
            let begun = Instant::now();
            let mut b = [0u8; 1];
            let n = extra.read(&mut b).unwrap_or(0);
            assert_eq!(n, 0, "closed");
            assert!(begun.elapsed() < Duration::from_millis(300), "{:?}", begun.elapsed());
        }
        assert!(wait(Duration::from_secs(5), || server.status().refused == 5), "{:?}", server.status().refused);
    }
}
