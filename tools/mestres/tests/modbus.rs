//! Core's Modbus/TCP client against the bench's Modbus server (contract: the Modbus client
//! clause; the bench's server is maquinar's own clause).
//!
//! The server is a whole device written apart from the client (maquinar's own frame code; the
//! client frames with tokio-modbus): its four tables set and read on the plant side, its knobs
//! making it slow, silent or restarted. Every function the client speaks is met here, every value
//! it reads is the one the plant side set, every value it writes is the one the plant side then
//! holds, and the server's own record of each exchange is the standard's bytes.

use std::time::{Duration, Instant};

use veredas_core::fieldbus::modbus::{Cause, Client, ModbusError, Target, Timeouts};
use veredas_maquinar::modbus::{self as bench, ModbusConfig, ModbusHandle, Table};

const BOUND: Duration = Duration::from_millis(300);
const TIMEOUTS: Timeouts = Timeouts {
    connect: Duration::from_millis(500),
    response: BOUND,
};

/// A device of 200 addresses in each table, answering `unit` (every unit when `None`).
fn device(unit: Option<u8>) -> ModbusHandle {
    let cfg = ModbusConfig {
        port: 0,
        unit,
        coils: Some(200),
        discrete_inputs: Some(200),
        input_registers: Some(200),
        holding_registers: Some(200),
        ..ModbusConfig::default()
    };
    bench::start(&cfg).unwrap()
}

fn client(dev: &ModbusHandle, unit: u8) -> Client {
    Client::new(
        Target {
            addr: dev.addr,
            unit,
        },
        TIMEOUTS,
    )
    .unwrap()
}

#[test]
fn every_function_reads_what_the_plant_holds_and_writes_what_it_then_holds() {
    let dev = device(None);
    let mut c = client(&dev, 1);
    // Reads: what the plant side set.
    let pattern: Vec<bool> = (0..19).map(|i| i % 3 == 0).collect();
    dev.set_bits(Table::Coil, 20, &pattern).unwrap();
    assert_eq!(c.read_coils(20, 19).unwrap(), pattern);
    let inputs: Vec<bool> = (0..22).map(|i| i % 2 == 1).collect();
    dev.set_bits(Table::DiscreteInput, 197 - 22, &inputs)
        .unwrap();
    assert_eq!(c.read_discrete_inputs(175, 22).unwrap(), inputs);
    dev.set_registers(Table::HoldingRegister, 107, &[555, 0, 100])
        .unwrap();
    assert_eq!(c.read_holding_registers(107, 3).unwrap(), vec![555, 0, 100]);
    dev.set_registers(Table::InputRegister, 8, &[10, 0xFFFF])
        .unwrap();
    assert_eq!(c.read_input_registers(8, 2).unwrap(), vec![10, 0xFFFF]);
    // Writes: what the plant side then holds.
    c.write_coil(172, true).unwrap();
    assert_eq!(dev.bits(Table::Coil, 172, 1).unwrap(), vec![true]);
    c.write_coil(172, false).unwrap();
    assert_eq!(dev.bits(Table::Coil, 172, 1).unwrap(), vec![false]);
    c.write_register(1, 3).unwrap();
    assert_eq!(
        dev.registers(Table::HoldingRegister, 1, 1).unwrap(),
        vec![3]
    );
    let ten: Vec<bool> = [1, 0, 1, 1, 0, 0, 1, 1, 1, 0]
        .iter()
        .map(|b| *b == 1)
        .collect();
    c.write_coils(19, &ten).unwrap();
    assert_eq!(dev.bits(Table::Coil, 19, 10).unwrap(), ten);
    c.write_registers(2, &[0x000A, 0x0102]).unwrap();
    assert_eq!(
        dev.registers(Table::HoldingRegister, 2, 2).unwrap(),
        vec![0x000A, 0x0102]
    );
    // The standard's mask: (current AND and) OR (or AND NOT and); 0x12 with 0xF2 and 0x25 is 0x17.
    dev.set_registers(Table::HoldingRegister, 4, &[0x0012])
        .unwrap();
    c.mask_write_register(4, 0x00F2, 0x0025).unwrap();
    assert_eq!(
        dev.registers(Table::HoldingRegister, 4, 1).unwrap(),
        vec![0x0017]
    );
    // Written, then read in one request: the read sees the write when the two overlap.
    dev.set_registers(Table::HoldingRegister, 3, &[0xFE, 0x0ACD, 1, 3, 0x0D, 0xFF])
        .unwrap();
    assert_eq!(
        c.read_write_registers(3, 6, 14, &[0xFF, 0xFF, 0xFF])
            .unwrap(),
        vec![0xFE, 0x0ACD, 1, 3, 0x0D, 0xFF]
    );
    assert_eq!(
        dev.registers(Table::HoldingRegister, 14, 3).unwrap(),
        vec![0xFF; 3]
    );
    assert_eq!(
        c.read_write_registers(14, 2, 15, &[7]).unwrap(),
        vec![0xFF, 7]
    );
    // The largest requests the standard allows, at the table's last addresses.
    let big = device(None);
    let mut b = client(&big, 1);
    let many: Vec<bool> = (0..200).map(|i| i % 7 == 0).collect();
    b.write_coils(0, &many).unwrap();
    assert_eq!(b.read_coils(0, 200).unwrap(), many);
    let words: Vec<u16> = (0..123).map(|i| i * 3).collect();
    b.write_registers(77, &words).unwrap();
    assert_eq!(b.read_holding_registers(75, 125).unwrap()[2..], words[..]);
    // One connection served them all.
    assert_eq!(dev.status().accepted, 1);
}

#[test]
fn the_bench_records_the_standards_bytes_for_each_request() {
    let dev = device(None);
    let mut c = client(&dev, 0x11);
    c.read_coils(0x13, 0x13).unwrap();
    c.read_discrete_inputs(0xC4 - 0xC0, 0x16).unwrap();
    c.read_holding_registers(0x6B, 3).unwrap();
    c.read_input_registers(0x08, 1).unwrap();
    c.write_coil(0xAC, true).unwrap();
    c.write_register(0x01, 0x0003).unwrap();
    let ten: Vec<bool> = [1, 0, 1, 1, 0, 0, 1, 1, 1, 0]
        .iter()
        .map(|b| *b == 1)
        .collect();
    c.write_coils(0x13, &ten).unwrap();
    c.write_registers(0x01, &[0x000A, 0x0102]).unwrap();
    c.mask_write_register(0x04, 0x00F2, 0x0025).unwrap();
    c.read_write_registers(0x03, 6, 0x0E, &[0x00FF, 0x00FF, 0x00FF])
        .unwrap();
    let seen: Vec<(u16, u8, Vec<u8>)> = dev
        .status()
        .recent
        .iter()
        .map(|e| (e.transaction, e.unit, e.request.clone()))
        .collect();
    let pdus: Vec<Vec<u8>> = vec![
        vec![0x01, 0x00, 0x13, 0x00, 0x13],
        vec![0x02, 0x00, 0x04, 0x00, 0x16],
        vec![0x03, 0x00, 0x6B, 0x00, 0x03],
        vec![0x04, 0x00, 0x08, 0x00, 0x01],
        vec![0x05, 0x00, 0xAC, 0xFF, 0x00],
        vec![0x06, 0x00, 0x01, 0x00, 0x03],
        vec![0x0F, 0x00, 0x13, 0x00, 0x0A, 0x02, 0xCD, 0x01],
        vec![0x10, 0x00, 0x01, 0x00, 0x02, 0x04, 0x00, 0x0A, 0x01, 0x02],
        vec![0x16, 0x00, 0x04, 0x00, 0xF2, 0x00, 0x25],
        vec![
            0x17, 0x00, 0x03, 0x00, 0x06, 0x00, 0x0E, 0x00, 0x03, 0x06, 0x00, 0xFF, 0x00, 0xFF,
            0x00, 0xFF,
        ],
    ];
    let want: Vec<(u16, u8, Vec<u8>)> = pdus
        .into_iter()
        .enumerate()
        .map(|(i, p)| (i as u16, 0x11, p))
        .collect();
    assert_eq!(seen, want);
    assert!(dev.status().recent.iter().all(|e| e.response.is_some()));
}

#[test]
fn the_devices_exceptions_come_back_as_its_no_and_the_connection_stays() {
    let dev = device(Some(1));
    let mut c = client(&dev, 1);
    // An address past the table's end: illegal data address.
    assert_eq!(
        c.read_holding_registers(199, 2).unwrap_err(),
        ModbusError::Exception {
            function: 0x03,
            code: 0x02
        }
    );
    assert_eq!(
        c.write_coils(195, &[true; 6]).unwrap_err(),
        ModbusError::Exception {
            function: 0x0F,
            code: 0x02
        }
    );
    assert_eq!(
        dev.bits(Table::Coil, 195, 5).unwrap(),
        vec![false; 5],
        "nothing written"
    );
    assert_eq!(c.read_input_registers(0, 1).unwrap(), vec![0]);
    assert_eq!(
        dev.status().accepted,
        1,
        "an exception keeps the connection"
    );
    // A unit the device does not play: the gateway's no.
    let mut other = client(&dev, 2);
    let e = other.read_coils(0, 1).unwrap_err();
    assert_eq!(
        e,
        ModbusError::Exception {
            function: 0x01,
            code: 0x0B
        }
    );
    assert_eq!(
        e.to_string(),
        "the device answered function 0x01 with exception 0x0b (gateway target device failed to respond)"
    );
    assert_eq!(dev.status().exceptions.get(&(0x01, 0x0B)), Some(&1));
}

#[test]
fn a_slow_device_lands_the_write_it_never_confirms_and_a_silent_one_does_not() {
    let dev = device(None);
    let mut c = client(&dev, 1);
    c.read_coils(0, 1).unwrap();
    // Slow: the device writes, then answers after the bound. The client cannot know; the plant
    // holds the value.
    dev.set_delay(BOUND * 2);
    let t = Instant::now();
    let e = c.write_register(10, 42).unwrap_err();
    assert_eq!(
        e,
        ModbusError::Unconfirmed {
            function: 0x06,
            cause: Cause::Timeout(BOUND)
        }
    );
    assert!(t.elapsed() < BOUND * 3, "{:?}", t.elapsed());
    assert_eq!(
        dev.registers(Table::HoldingRegister, 10, 1).unwrap(),
        vec![42],
        "it landed"
    );
    assert!(!c.is_connected());
    // A read gets nothing; its late answer is never taken for the next request's.
    dev.set_registers(Table::InputRegister, 0, &[1, 2]).unwrap();
    assert_eq!(
        c.read_input_registers(0, 1).unwrap_err(),
        ModbusError::NoAnswer {
            function: 0x04,
            cause: Cause::Timeout(BOUND)
        }
    );
    dev.set_delay(Duration::ZERO);
    assert_eq!(c.read_input_registers(1, 1).unwrap(), vec![2]);
    // Silent: the device reads the request and does nothing with it. The same error, and this
    // time the plant does not hold the value: "unconfirmed" is both.
    dev.set_silent(true);
    let e = c.write_register(11, 43).unwrap_err();
    assert_eq!(
        e,
        ModbusError::Unconfirmed {
            function: 0x06,
            cause: Cause::Timeout(BOUND)
        }
    );
    assert_eq!(
        dev.registers(Table::HoldingRegister, 11, 1).unwrap(),
        vec![0],
        "it did not land"
    );
    dev.set_silent(false);
    c.write_register(11, 43).unwrap();
    assert_eq!(
        dev.registers(Table::HoldingRegister, 11, 1).unwrap(),
        vec![43]
    );
    let status = dev.status();
    assert_eq!(status.unanswered, 1);
    assert_eq!(
        status.accepted, 4,
        "each unanswered call ended its connection"
    );
}

#[test]
fn a_restarted_device_ends_the_call_and_the_next_one_connects_again() {
    let dev = device(None);
    let mut c = client(&dev, 1);
    dev.set_registers(Table::HoldingRegister, 0, &[7]).unwrap();
    assert_eq!(c.read_holding_registers(0, 1).unwrap(), vec![7]);
    dev.drop_connections();
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
    assert_eq!(c.read_holding_registers(0, 1).unwrap(), vec![7]);
    dev.drop_connections();
    let e = c.write_register(0, 8).unwrap_err();
    assert!(
        matches!(
            e,
            ModbusError::Unconfirmed {
                function: 0x06,
                cause: Cause::Link(_)
            }
        ),
        "{e:?}"
    );
    assert_eq!(
        dev.registers(Table::HoldingRegister, 0, 1).unwrap(),
        vec![7],
        "never reached it"
    );
    c.write_register(0, 8).unwrap();
    assert_eq!(
        dev.registers(Table::HoldingRegister, 0, 1).unwrap(),
        vec![8]
    );
    assert_eq!(dev.status().accepted, 3);
    // A device gone altogether: no connection, nothing sent.
    let addr = dev.addr;
    dev.stop();
    let mut gone = Client::new(Target { addr, unit: 1 }, TIMEOUTS).unwrap();
    assert!(matches!(
        gone.write_coil(0, true),
        Err(ModbusError::NotConnected { .. })
    ));
}
