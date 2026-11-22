//! The Modbus/TCP client (contract: the Modbus client clause), over real TCP on loopback.
//!
//! The peer here is a socket the test drives byte by byte: it records every request as it
//! arrives on the wire and answers with the bytes each test gives it, or with silence, a close,
//! an answer too late. The requests and answers of every function are the Modbus Application
//! Protocol specification's own examples (V1.1b3, its examples of each function code), so what
//! the client sends and what it reads are held to the standard's bytes, not to this crate's
//! reading of them. The same client against a whole device, the bench's Modbus server, is the
//! clause's other test (`docs/questions.md` Q101).

use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use veredas_core::fieldbus::modbus::*;

/// What the peer does with one request.
#[derive(Clone)]
enum Reply {
    /// These bytes, at once.
    Send(Vec<u8>),
    /// Nothing; the connection stays open.
    Silent,
    /// Closes the connection.
    Close,
    /// These bytes, after a while.
    Late(Duration, Vec<u8>),
    /// These bytes, cut after `n` and the connection then closed.
    Cut(usize, Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Seen {
    Connected(usize),
    Request(usize, Vec<u8>),
    Closed(usize),
}

type Script = Arc<dyn Fn(usize, &[u8]) -> Reply + Send + Sync>;

/// A Modbus/TCP peer on loopback, scripted per request: `script(request number, request ADU)`.
struct Peer {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<Seen>>>,
    stop: Arc<AtomicBool>,
}

impl Peer {
    fn start(script: impl Fn(usize, &[u8]) -> Reply + Send + Sync + 'static) -> Peer {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let script: Script = Arc::new(script);
        let requests = Arc::new(Mutex::new(0usize));
        let (seen2, stop2) = (seen.clone(), stop.clone());
        thread::spawn(move || {
            for (n, conn) in listener.incoming().enumerate() {
                if stop2.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(mut conn) = conn else { continue };
                seen2.lock().unwrap().push(Seen::Connected(n));
                let (seen, script, requests) = (seen2.clone(), script.clone(), requests.clone());
                thread::spawn(move || {
                    loop {
                        let mut header = [0u8; 7];
                        if conn.read_exact(&mut header).is_err() {
                            break;
                        }
                        let len = u16::from_be_bytes([header[4], header[5]]) as usize;
                        let mut pdu = vec![0u8; len.saturating_sub(1)];
                        if conn.read_exact(&mut pdu).is_err() {
                            break;
                        }
                        let adu = [header.to_vec(), pdu].concat();
                        seen.lock().unwrap().push(Seen::Request(n, adu.clone()));
                        let index = {
                            let mut r = requests.lock().unwrap();
                            *r += 1;
                            *r - 1
                        };
                        match script(index, &adu) {
                            Reply::Send(bytes) => {
                                let _ = conn.write_all(&bytes);
                            }
                            Reply::Silent => {}
                            Reply::Close => {
                                let _ = conn.shutdown(Shutdown::Both);
                                break;
                            }
                            Reply::Late(after, bytes) => {
                                thread::sleep(after);
                                let _ = conn.write_all(&bytes);
                            }
                            Reply::Cut(n, bytes) => {
                                let _ = conn.write_all(&bytes[..n]);
                                let _ = conn.shutdown(Shutdown::Both);
                                break;
                            }
                        }
                    }
                    seen.lock().unwrap().push(Seen::Closed(n));
                });
            }
        });
        Peer { addr, seen, stop }
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    fn requests(&self) -> Vec<Vec<u8>> {
        self.seen()
            .into_iter()
            .filter_map(|s| match s {
                Seen::Request(_, adu) => Some(adu),
                _ => None,
            })
            .collect()
    }

    fn connections(&self) -> usize {
        self.seen()
            .iter()
            .filter(|s| matches!(s, Seen::Connected(_)))
            .count()
    }

    /// Waits up to a second for the peer to see connection `n` close.
    fn closed(&self, n: usize) -> bool {
        let until = Instant::now() + Duration::from_secs(1);
        while Instant::now() < until {
            if self.seen().contains(&Seen::Closed(n)) {
                return true;
            }
            thread::sleep(Duration::from_millis(5));
        }
        false
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr);
    }
}

/// An ADU: the MBAP header (transaction, protocol 0, length, unit) and the PDU.
fn adu(transaction: u16, unit: u8, pdu: &[u8]) -> Vec<u8> {
    let mut out = transaction.to_be_bytes().to_vec();
    out.extend_from_slice(&[0, 0]);
    out.extend_from_slice(&(pdu.len() as u16 + 1).to_be_bytes());
    out.push(unit);
    out.extend_from_slice(pdu);
    out
}

/// The answer to a request: its own transaction and unit, this PDU.
fn answer(request: &[u8], pdu: &[u8]) -> Vec<u8> {
    adu(
        u16::from_be_bytes([request[0], request[1]]),
        request[6],
        pdu,
    )
}

const FAST: Timeouts = Timeouts {
    connect: Duration::from_millis(500),
    response: Duration::from_millis(300),
};

fn client(peer: &Peer, unit: u8) -> Client {
    Client::new(
        Target {
            addr: peer.addr,
            unit,
        },
        FAST,
    )
    .unwrap()
}

/// The specification's example of each function: the request PDU and the answer PDU.
struct Example {
    request: &'static [u8],
    answer: &'static [u8],
}

const READ_COILS: Example = Example {
    request: &[0x01, 0x00, 0x13, 0x00, 0x13],
    answer: &[0x01, 0x03, 0xCD, 0x6B, 0x05],
};
const READ_DISCRETE_INPUTS: Example = Example {
    request: &[0x02, 0x00, 0xC4, 0x00, 0x16],
    answer: &[0x02, 0x03, 0xAC, 0xDB, 0x35],
};
const READ_HOLDING_REGISTERS: Example = Example {
    request: &[0x03, 0x00, 0x6B, 0x00, 0x03],
    answer: &[0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64],
};
const READ_INPUT_REGISTERS: Example = Example {
    request: &[0x04, 0x00, 0x08, 0x00, 0x01],
    answer: &[0x04, 0x02, 0x00, 0x0A],
};
const WRITE_SINGLE_COIL: Example = Example {
    request: &[0x05, 0x00, 0xAC, 0xFF, 0x00],
    answer: &[0x05, 0x00, 0xAC, 0xFF, 0x00],
};
const WRITE_SINGLE_REGISTER: Example = Example {
    request: &[0x06, 0x00, 0x01, 0x00, 0x03],
    answer: &[0x06, 0x00, 0x01, 0x00, 0x03],
};
const WRITE_MULTIPLE_COILS: Example = Example {
    request: &[0x0F, 0x00, 0x13, 0x00, 0x0A, 0x02, 0xCD, 0x01],
    answer: &[0x0F, 0x00, 0x13, 0x00, 0x0A],
};
const WRITE_MULTIPLE_REGISTERS: Example = Example {
    request: &[0x10, 0x00, 0x01, 0x00, 0x02, 0x04, 0x00, 0x0A, 0x01, 0x02],
    answer: &[0x10, 0x00, 0x01, 0x00, 0x02],
};
const MASK_WRITE_REGISTER: Example = Example {
    request: &[0x16, 0x00, 0x04, 0x00, 0xF2, 0x00, 0x25],
    answer: &[0x16, 0x00, 0x04, 0x00, 0xF2, 0x00, 0x25],
};
const READ_WRITE_MULTIPLE_REGISTERS: Example = Example {
    request: &[
        0x17, 0x00, 0x03, 0x00, 0x06, 0x00, 0x0E, 0x00, 0x03, 0x06, 0x00, 0xFF, 0x00, 0xFF, 0x00,
        0xFF,
    ],
    answer: &[
        0x17, 0x0C, 0x00, 0xFE, 0x0A, 0xCD, 0x00, 0x01, 0x00, 0x03, 0x00, 0x0D, 0x00, 0xFF,
    ],
};

/// A peer that answers each request with the example whose request PDU it is, or an
/// illegal-function exception.
fn standard_device() -> Peer {
    let examples = [
        READ_COILS,
        READ_DISCRETE_INPUTS,
        READ_HOLDING_REGISTERS,
        READ_INPUT_REGISTERS,
        WRITE_SINGLE_COIL,
        WRITE_SINGLE_REGISTER,
        WRITE_MULTIPLE_COILS,
        WRITE_MULTIPLE_REGISTERS,
        MASK_WRITE_REGISTER,
        READ_WRITE_MULTIPLE_REGISTERS,
    ];
    Peer::start(move |_, req| {
        let pdu = &req[7..];
        match examples.iter().find(|e| e.request == pdu) {
            Some(e) => Reply::Send(answer(req, e.answer)),
            None => Reply::Send(answer(req, &[pdu[0] | 0x80, 0x01])),
        }
    })
}

fn bits(pattern: &str) -> Vec<bool> {
    pattern
        .chars()
        .filter(|c| *c != ' ')
        .map(|c| c == '1')
        .collect()
}

#[test]
fn every_function_sends_the_standards_bytes_and_reads_its_answer() {
    let peer = standard_device();
    let mut c = client(&peer, 0x11);
    // Coils 20 to 38: 0xCD 0x6B 0x05, least significant bit first.
    assert_eq!(
        c.read_coils(0x13, 0x13).unwrap(),
        bits("10110011 11010110 101")
    );
    // Inputs 197 to 218: 0xAC 0xDB 0x35.
    assert_eq!(
        c.read_discrete_inputs(0xC4, 0x16).unwrap(),
        bits("00110101 11011011 101011")
    );
    assert_eq!(
        c.read_holding_registers(0x6B, 3).unwrap(),
        vec![0x022B, 0x0000, 0x0064]
    );
    assert_eq!(c.read_input_registers(0x08, 1).unwrap(), vec![0x000A]);
    c.write_coil(0xAC, true).unwrap();
    c.write_register(0x01, 0x0003).unwrap();
    c.write_coils(0x13, &bits("10110011 10")).unwrap();
    c.write_registers(0x01, &[0x000A, 0x0102]).unwrap();
    c.mask_write_register(0x04, 0x00F2, 0x0025).unwrap();
    assert_eq!(
        c.read_write_registers(0x03, 6, 0x0E, &[0x00FF, 0x00FF, 0x00FF])
            .unwrap(),
        vec![0x00FE, 0x0ACD, 0x0001, 0x0003, 0x000D, 0x00FF]
    );
    // On the wire, each request is the example's PDU behind an MBAP header: a transaction number
    // counting up from 0, protocol 0, the length, the unit.
    let sent = peer.requests();
    let examples = [
        READ_COILS,
        READ_DISCRETE_INPUTS,
        READ_HOLDING_REGISTERS,
        READ_INPUT_REGISTERS,
        WRITE_SINGLE_COIL,
        WRITE_SINGLE_REGISTER,
        WRITE_MULTIPLE_COILS,
        WRITE_MULTIPLE_REGISTERS,
        MASK_WRITE_REGISTER,
        READ_WRITE_MULTIPLE_REGISTERS,
    ];
    let want: Vec<Vec<u8>> = examples
        .iter()
        .enumerate()
        .map(|(i, e)| adu(i as u16, 0x11, e.request))
        .collect();
    assert_eq!(sent, want);
    assert_eq!(peer.connections(), 1, "one connection for every request");
    assert!(c.is_connected());
}

#[test]
fn an_exception_is_the_devices_no_and_the_connection_stays() {
    let peer = Peer::start(|i, req| match i {
        0 => Reply::Send(answer(req, &[0x81, 0x02])),
        1 => Reply::Send(answer(req, &[0x83, 0x0B])),
        _ => Reply::Send(answer(req, &[0x03, 0x02, 0x00, 0x07])),
    });
    let mut c = client(&peer, 1);
    let e = c.read_coils(0, 1).unwrap_err();
    assert_eq!(
        e,
        ModbusError::Exception {
            function: 0x01,
            code: 0x02
        }
    );
    assert_eq!(
        e.to_string(),
        "the device answered function 0x01 with exception 0x02 (illegal data address)"
    );
    assert_eq!(
        c.read_holding_registers(0, 1).unwrap_err().to_string(),
        "the device answered function 0x03 with exception 0x0b (gateway target device failed to respond)"
    );
    assert_eq!(c.read_holding_registers(0, 1).unwrap(), vec![7]);
    assert_eq!(peer.connections(), 1);
    assert_eq!(exception_name(0x63), "not a standard exception");
}

/// A write's exception is the device's no only when the standard says the request was not
/// carried out; otherwise the write may have landed, and the connection stays either way.
#[test]
fn a_writes_exception_says_not_done_only_where_the_standard_does() {
    // Every code the standard names, two it does not; each on every write, then on a read.
    let codes: [u8; 11] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x08, 0x0A, 0x0B, 0x00, 0x63,
    ];
    let refused: Vec<u8> = codes.iter().copied().filter(|c| refuses(*c)).collect();
    assert_eq!(refused, vec![0x01, 0x02, 0x03, 0x06, 0x0A]);
    let peer = Peer::start(move |i, req| {
        let code = codes[i % codes.len()];
        Reply::Send(answer(req, &[req[7] | 0x80, code]))
    });
    let mut c = client(&peer, 1);
    type Write = fn(&mut Client) -> Result<(), ModbusError>;
    let writes: [(u8, Write); 6] = [
        (0x05, |c| c.write_coil(1, true)),
        (0x06, |c| c.write_register(1, 2)),
        (0x0F, |c| c.write_coils(0, &[true; 3])),
        (0x10, |c| c.write_registers(0, &[1, 2, 3])),
        (0x16, |c| c.mask_write_register(4, 0xF2, 0x25)),
        (0x17, |c| c.read_write_registers(0, 1, 0, &[1]).map(|_| ())),
    ];
    for (function, write) in writes {
        for code in codes {
            let e = write(&mut c).unwrap_err();
            let want = if refuses(code) {
                ModbusError::Exception { function, code }
            } else {
                ModbusError::Unconfirmed {
                    function,
                    cause: Cause::Exception(code),
                }
            };
            assert_eq!(e, want, "function {function:#04x}, exception {code:#04x}");
        }
    }
    // The exact words of the two kinds.
    assert_eq!(
        Cause::Exception(0x05).to_string(),
        "the device answered exception 0x05 (acknowledge)"
    );
    assert_eq!(
        ModbusError::Unconfirmed {
            function: 0x0F,
            cause: Cause::Exception(0x0B)
        }
        .to_string(),
        "function 0x0f unconfirmed, it may have landed: the device answered exception 0x0b (gateway target device failed to respond)"
    );
    // A read's exception is always its no: no data came, and a read changes nothing.
    for code in codes {
        assert_eq!(
            c.read_holding_registers(0, 1).unwrap_err(),
            ModbusError::Exception {
                function: 0x03,
                code
            }
        );
    }
    assert!(c.is_connected());
    assert_eq!(
        peer.connections(),
        1,
        "an exception keeps the connection, whatever its code"
    );
}

#[test]
fn a_silent_device_ends_the_call_at_the_bound_and_the_connection_with_it() {
    let peer = Peer::start(|i, req| match i {
        0 | 1 => Reply::Silent,
        _ => Reply::Send(answer(req, &[0x03, 0x02, 0x00, 0x2A])),
    });
    let mut c = client(&peer, 1);
    let t = Instant::now();
    let e = c.read_holding_registers(0, 1).unwrap_err();
    let took = t.elapsed();
    assert_eq!(
        e,
        ModbusError::NoAnswer {
            function: 0x03,
            cause: Cause::Timeout(Duration::from_millis(300))
        }
    );
    assert_eq!(
        e.to_string(),
        "function 0x03 got no answer: no answer within 300 ms"
    );
    assert!(
        took >= Duration::from_millis(300) && took < Duration::from_millis(900),
        "{took:?}"
    );
    assert!(!c.is_connected());
    assert!(
        peer.closed(0),
        "the connection is closed, not left to deliver late"
    );
    // A write unanswered may have landed.
    let e = c.write_register(5, 9).unwrap_err();
    assert_eq!(
        e,
        ModbusError::Unconfirmed {
            function: 0x06,
            cause: Cause::Timeout(Duration::from_millis(300))
        }
    );
    assert_eq!(
        e.to_string(),
        "function 0x06 unconfirmed, it may have landed: no answer within 300 ms"
    );
    // The next call connects again.
    assert_eq!(c.read_holding_registers(0, 1).unwrap(), vec![42]);
    assert_eq!(peer.connections(), 3);
}

#[test]
fn a_late_answer_is_never_read_as_the_next_requests() {
    // The first answer comes after the bound, on the connection the client has closed; the second
    // request, on a new connection, must read its own answer.
    let peer = Peer::start(|i, req| match i {
        0 => Reply::Late(
            Duration::from_millis(450),
            answer(req, &[0x03, 0x02, 0x00, 0x01]),
        ),
        _ => Reply::Late(
            Duration::from_millis(200),
            answer(req, &[0x03, 0x02, 0x00, 0x02]),
        ),
    });
    let mut c = client(&peer, 1);
    assert!(matches!(
        c.read_holding_registers(0, 1),
        Err(ModbusError::NoAnswer { .. })
    ));
    assert_eq!(c.read_holding_registers(0, 1).unwrap(), vec![2]);
}

#[test]
fn a_broken_link_ends_the_call_and_a_write_is_unconfirmed() {
    let peer = Peer::start(|i, req| match i {
        0 | 2 => Reply::Close,
        1 => Reply::Cut(9, answer(req, &[0x03, 0x02, 0x00, 0x01])),
        3 => Reply::Cut(7, answer(req, &[0x10, 0x00, 0x00, 0x00, 0x01])),
        _ => Reply::Send(answer(req, &[0x01, 0x01, 0x01])),
    });
    let mut c = client(&peer, 1);
    for _ in 0..2 {
        let e = c.read_holding_registers(0, 1).unwrap_err();
        assert!(
            matches!(
                e,
                ModbusError::NoAnswer {
                    function: 0x03,
                    cause: Cause::Link(_)
                }
            ),
            "{e:?}"
        );
        assert!(!c.is_connected());
    }
    for _ in 0..2 {
        let e = c.write_registers(0, &[1]).unwrap_err();
        assert!(
            matches!(
                e,
                ModbusError::Unconfirmed {
                    function: 0x10,
                    cause: Cause::Link(_)
                }
            ),
            "{e:?}"
        );
    }
    assert_eq!(c.read_coils(0, 1).unwrap(), vec![true]);
    assert_eq!(peer.connections(), 5);
}

#[test]
fn an_answer_that_does_not_answer_the_request_is_refused_and_ends_the_connection() {
    // Each script answers request i wrongly in one way, then the client must reconnect.
    type Wrong = fn(&[u8]) -> Vec<u8>;
    type Call = fn(&mut Client) -> Result<(), ModbusError>;
    let wrongs: Vec<(&str, Wrong, Call, bool)> = vec![
        // An exception is an answer too, held to its request like any other: one that answers
        // another request must not be read as this write's "not done" nor keep the connection.
        (
            "a write's exception for another function",
            |req| answer(req, &[0x83, 0x02]),
            |c| c.write_register(1, 2),
            true,
        ),
        (
            "a write's exception for another transaction",
            |req| {
                adu(
                    u16::from_be_bytes([req[0], req[1]]) ^ 0x5555,
                    req[6],
                    &[0x86, 0x02],
                )
            },
            |c| c.write_register(1, 2),
            true,
        ),
        (
            "a write's exception for another unit",
            |req| {
                adu(
                    u16::from_be_bytes([req[0], req[1]]),
                    req[6] ^ 1,
                    &[0x86, 0x02],
                )
            },
            |c| c.write_register(1, 2),
            true,
        ),
        (
            "a read's exception for another function",
            |req| answer(req, &[0x84, 0x02]),
            |c| c.read_holding_registers(0, 1).map(|_| ()),
            false,
        ),
        (
            "a read's exception for another transaction",
            |req| {
                adu(
                    u16::from_be_bytes([req[0], req[1]]) ^ 0x5555,
                    req[6],
                    &[0x83, 0x02],
                )
            },
            |c| c.read_holding_registers(0, 1).map(|_| ()),
            false,
        ),
        (
            "a read's exception for another unit",
            |req| {
                adu(
                    u16::from_be_bytes([req[0], req[1]]),
                    req[6] ^ 1,
                    &[0x83, 0x02],
                )
            },
            |c| c.read_holding_registers(0, 1).map(|_| ()),
            false,
        ),
        (
            "another transaction",
            |req| {
                adu(
                    u16::from_be_bytes([req[0], req[1]]) ^ 0x5555,
                    req[6],
                    &[0x03, 0x02, 0, 1],
                )
            },
            |c| c.read_holding_registers(0, 1).map(|_| ()),
            false,
        ),
        (
            "another unit",
            |req| {
                adu(
                    u16::from_be_bytes([req[0], req[1]]),
                    req[6] ^ 1,
                    &[0x03, 0x02, 0, 1],
                )
            },
            |c| c.read_holding_registers(0, 1).map(|_| ()),
            false,
        ),
        (
            "another function",
            |req| answer(req, &[0x04, 0x02, 0, 1]),
            |c| c.read_holding_registers(0, 1).map(|_| ()),
            false,
        ),
        (
            "fewer registers",
            |req| answer(req, &[0x03, 0x02, 0, 1]),
            |c| c.read_holding_registers(0, 2).map(|_| ()),
            false,
        ),
        (
            "more registers",
            |req| answer(req, &[0x03, 0x04, 0, 1, 0, 2]),
            |c| c.read_holding_registers(0, 1).map(|_| ()),
            false,
        ),
        (
            "too few bytes of bits",
            |req| answer(req, &[0x01, 0x01, 0xFF]),
            |c| c.read_coils(0, 9).map(|_| ()),
            false,
        ),
        (
            "too many bytes of bits",
            |req| answer(req, &[0x02, 0x02, 0xFF, 0x00]),
            |c| c.read_discrete_inputs(0, 8).map(|_| ()),
            false,
        ),
        (
            "an echo of another address",
            |req| answer(req, &[0x06, 0x00, 0x02, 0x00, 0x09]),
            |c| c.write_register(1, 9),
            true,
        ),
        (
            "an echo of another value",
            |req| answer(req, &[0x05, 0x00, 0x01, 0x00, 0x00]),
            |c| c.write_coil(1, true),
            true,
        ),
        (
            "an echo of another quantity",
            |req| answer(req, &[0x0F, 0x00, 0x00, 0x00, 0x02]),
            |c| c.write_coils(0, &[true, false, true]),
            true,
        ),
        (
            "an echo of another mask",
            |req| answer(req, &[0x16, 0x00, 0x04, 0x00, 0xF2, 0x00, 0x24]),
            |c| c.mask_write_register(4, 0xF2, 0x25),
            true,
        ),
        (
            "fewer registers read by a write",
            |req| answer(req, &[0x17, 0x02, 0x00, 0x01]),
            |c| c.read_write_registers(0, 2, 0, &[1]).map(|_| ()),
            true,
        ),
        (
            "a coil echoed as neither on nor off",
            |req| answer(req, &[0x05, 0x00, 0x01, 0x12, 0x34]),
            |c| c.write_coil(1, true),
            true,
        ),
    ];
    for (what, wrong, call, write) in wrongs {
        let peer = Peer::start(move |i, req| match i {
            0 => Reply::Send(wrong(req)),
            _ => Reply::Send(answer(req, &[0x03, 0x02, 0x00, 0x05])),
        });
        let mut c = client(&peer, 3);
        let e = call(&mut c).unwrap_err();
        let ok = if write {
            matches!(
                e,
                ModbusError::Unconfirmed {
                    cause: Cause::Mismatch(_) | Cause::Link(_),
                    ..
                }
            )
        } else {
            matches!(
                e,
                ModbusError::NoAnswer {
                    cause: Cause::Mismatch(_) | Cause::Link(_),
                    ..
                }
            )
        };
        assert!(ok, "{what}: {e:?}");
        assert!(!c.is_connected(), "{what}: the connection is closed");
        assert!(peer.closed(0), "{what}: the peer sees it close");
        assert_eq!(c.read_holding_registers(0, 1).unwrap(), vec![5], "{what}");
        assert_eq!(peer.connections(), 2, "{what}");
    }
}

#[test]
fn a_request_outside_the_standards_bounds_never_reaches_the_wire() {
    let peer = standard_device();
    let mut c = client(&peer, 1);
    let refused = |r: Result<(), ModbusError>| match r {
        Err(ModbusError::Refused { reason }) => reason,
        other => panic!("{other:?}"),
    };
    // Every function with a quantity, each of its bounds: none, one past its limit, a range past
    // the last address.
    let reasons = vec![
        refused(c.read_coils(0, 0).map(|_| ())),
        refused(c.read_coils(0, 2001).map(|_| ())),
        refused(c.read_coils(65535, 2).map(|_| ())),
        refused(c.read_discrete_inputs(0, 0).map(|_| ())),
        refused(c.read_discrete_inputs(0, 2001).map(|_| ())),
        refused(c.read_discrete_inputs(65535, 2).map(|_| ())),
        refused(c.read_holding_registers(0, 0).map(|_| ())),
        refused(c.read_holding_registers(0, 126).map(|_| ())),
        refused(c.read_holding_registers(65535, 2).map(|_| ())),
        refused(c.read_input_registers(0, 0).map(|_| ())),
        refused(c.read_input_registers(0, 126).map(|_| ())),
        refused(c.read_input_registers(65500, 37).map(|_| ())),
        refused(c.write_coils(0, &[])),
        refused(c.write_coils(0, &vec![true; 1969])),
        refused(c.write_coils(65535, &[true, true])),
        refused(c.write_registers(0, &[])),
        refused(c.write_registers(0, &vec![0; 124])),
        refused(c.write_registers(65535, &[1, 2])),
        refused(c.read_write_registers(0, 0, 0, &[1]).map(|_| ())),
        refused(c.read_write_registers(0, 126, 0, &[1]).map(|_| ())),
        refused(c.read_write_registers(65535, 2, 0, &[1]).map(|_| ())),
        refused(c.read_write_registers(0, 1, 0, &[]).map(|_| ())),
        refused(c.read_write_registers(0, 1, 0, &vec![0; 122]).map(|_| ())),
        refused(c.read_write_registers(0, 1, 65535, &[1, 2]).map(|_| ())),
    ];
    assert_eq!(
        reasons,
        vec![
            "0 coils read: the standard takes 1 to 2000",
            "2001 coils read: the standard takes 1 to 2000",
            "2 coils read from 65535 run past address 65535",
            "0 discrete inputs read: the standard takes 1 to 2000",
            "2001 discrete inputs read: the standard takes 1 to 2000",
            "2 discrete inputs read from 65535 run past address 65535",
            "0 holding registers read: the standard takes 1 to 125",
            "126 holding registers read: the standard takes 1 to 125",
            "2 holding registers read from 65535 run past address 65535",
            "0 input registers read: the standard takes 1 to 125",
            "126 input registers read: the standard takes 1 to 125",
            "37 input registers read from 65500 run past address 65535",
            "0 coils written: the standard takes 1 to 1968",
            "1969 coils written: the standard takes 1 to 1968",
            "2 coils written from 65535 run past address 65535",
            "0 registers written: the standard takes 1 to 123",
            "124 registers written: the standard takes 1 to 123",
            "2 registers written from 65535 run past address 65535",
            "0 registers read: the standard takes 1 to 125",
            "126 registers read: the standard takes 1 to 125",
            "2 registers read from 65535 run past address 65535",
            "0 registers written: the standard takes 1 to 121",
            "122 registers written: the standard takes 1 to 121",
            "2 registers written from 65535 run past address 65535",
        ]
    );
    assert_eq!(peer.connections(), 0, "nothing sent, not even a connection");
    // Every limit taken, at the last addresses it can reach: the largest quantity of each
    // function ending on address 65535, and one item at 65535.
    let peer = Peer::start(|_, req| {
        let pdu = &req[7..];
        let count = u16::from_be_bytes([pdu[3], pdu[4]]);
        match pdu[0] {
            f @ (0x01 | 0x02) => {
                let bytes = count.div_ceil(8) as usize;
                let mut a = vec![f, bytes as u8];
                a.resize(2 + bytes, 0);
                Reply::Send(answer(req, &a))
            }
            f @ (0x03 | 0x04 | 0x17) => {
                let mut a = vec![f, (count * 2) as u8];
                a.resize(2 + count as usize * 2, 0);
                Reply::Send(answer(req, &a))
            }
            0x0F | 0x10 => Reply::Send(answer(req, &pdu[..5])),
            _ => Reply::Send(answer(req, &[pdu[0] | 0x80, 1])),
        }
    });
    let mut c = client(&peer, 1);
    assert_eq!(c.read_coils(65535, 1).unwrap(), vec![false]);
    assert_eq!(c.read_coils(63536, 2000).unwrap().len(), 2000);
    assert_eq!(c.read_discrete_inputs(63536, 2000).unwrap().len(), 2000);
    assert_eq!(c.read_holding_registers(65411, 125).unwrap().len(), 125);
    assert_eq!(c.read_input_registers(65411, 125).unwrap().len(), 125);
    c.write_coils(63568, &vec![true; 1968]).unwrap();
    c.write_registers(65413, &[7; 123]).unwrap();
    assert_eq!(
        c.read_write_registers(65411, 125, 65415, &[7; 121])
            .unwrap()
            .len(),
        125
    );
    assert_eq!(peer.connections(), 1);
}

#[test]
fn connecting_is_bounded_and_says_nothing_was_sent() {
    // A port nobody listens on: refused at once.
    let closed = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let mut c = Client::new(
        Target {
            addr: closed,
            unit: 1,
        },
        FAST,
    )
    .unwrap();
    let t = Instant::now();
    let e = c.read_coils(0, 1).unwrap_err();
    assert!(
        matches!(e, ModbusError::NotConnected { addr, .. } if addr == closed),
        "{e:?}"
    );
    assert!(t.elapsed() < Duration::from_millis(500));
    assert!(
        e.to_string()
            .starts_with(&format!("no connection to {closed}: ")),
        "{e}"
    );
    // A write that never connected never left: not sent, not unconfirmed.
    assert!(matches!(
        c.write_coil(0, true),
        Err(ModbusError::NotConnected { .. })
    ));
    // A listener whose queue is full drops the connection's first packet: the connect hangs,
    // and the bound ends it.
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::STREAM, None).unwrap();
    socket
        .bind(&"127.0.0.1:0".parse::<SocketAddr>().unwrap().into())
        .unwrap();
    socket.listen(0).unwrap();
    let full = socket.local_addr().unwrap().as_socket().unwrap();
    let held: Vec<_> = (0..4)
        .filter_map(|_| TcpStream::connect_timeout(&full, Duration::from_millis(100)).ok())
        .collect();
    let mut c = Client::new(
        Target {
            addr: full,
            unit: 1,
        },
        FAST,
    )
    .unwrap();
    let t = Instant::now();
    let e = c.read_holding_registers(0, 1).unwrap_err();
    let took = t.elapsed();
    assert_eq!(
        e,
        ModbusError::NotConnected {
            addr: full,
            reason: "no connection within 500 ms".to_string()
        }
    );
    assert!(
        took >= Duration::from_millis(500) && took < Duration::from_millis(1100),
        "{took:?}"
    );
    drop(held);
}

#[test]
fn dropping_the_client_closes_its_connection() {
    let peer = standard_device();
    let mut c = client(&peer, 1);
    c.read_input_registers(0x08, 1).unwrap();
    assert!(!peer.seen().contains(&Seen::Closed(0)));
    drop(c);
    assert!(peer.closed(0));
    // And disconnect does the same, the next call connecting again.
    let mut c = client(&peer, 1);
    c.read_input_registers(0x08, 1).unwrap();
    c.disconnect();
    assert!(peer.closed(1));
    c.read_input_registers(0x08, 1).unwrap();
    assert_eq!(peer.connections(), 3);
    assert!(format!("{c:?}").contains("connected: true"));
}

#[test]
fn every_request_carries_the_targets_unit() {
    let peer = standard_device();
    for unit in [0u8, 1, 0x11, 0xF7, 0xFF] {
        let mut c = client(&peer, unit);
        c.read_input_registers(0x08, 1).unwrap();
    }
    let units: Vec<u8> = peer.requests().iter().map(|a| a[6]).collect();
    assert_eq!(units, vec![0, 1, 0x11, 0xF7, 0xFF]);
    assert_eq!(
        Timeouts::default(),
        Timeouts {
            connect: Duration::from_secs(3),
            response: Duration::from_secs(1)
        }
    );
    assert_eq!(DEFAULT_PORT, 502);
}
