//! The emulated EtherCAT segment's core: SubDevices as ESC memories, an EtherCAT frame's datagrams
//! processed through them in order as the frame passes, each command addressed, read, written and
//! counted as the standard has it. Pure: no socket, no clock but the one handed in.

/// An EtherCAT frame's EtherType.
pub const ETHERTYPE: u16 = 0x88A4;

/// The commands.
pub mod cmd {
    pub const NOP: u8 = 0;
    pub const APRD: u8 = 1;
    pub const APWR: u8 = 2;
    pub const APRW: u8 = 3;
    pub const FPRD: u8 = 4;
    pub const FPWR: u8 = 5;
    pub const FPRW: u8 = 6;
    pub const BRD: u8 = 7;
    pub const BWR: u8 = 8;
    pub const BRW: u8 = 9;
    pub const LRD: u8 = 10;
    pub const LWR: u8 = 11;
    pub const LRW: u8 = 12;
    pub const ARMW: u8 = 13;
    pub const FRMW: u8 = 14;
}

/// Registers with a behaviour of their own.
pub mod reg {
    pub const TYPE: u16 = 0x0000;
    pub const FMMUS_SUPPORTED: u16 = 0x0004;
    pub const SMS_SUPPORTED: u16 = 0x0005;
    pub const RAM_SIZE: u16 = 0x0006;
    pub const PORT_DESCRIPTOR: u16 = 0x0007;
    pub const FEATURES: u16 = 0x0008;
    pub const STATION_ADDRESS: u16 = 0x0010;
    pub const STATION_ALIAS: u16 = 0x0012;
    /// Byte 3 of the DL control: bit 0 lets the configured-address commands match the alias too.
    pub const DL_CONTROL_ALIAS: u16 = 0x0103;
    pub const DL_STATUS: u16 = 0x0110;
    pub const AL_CONTROL: u16 = 0x0120;
    pub const AL_STATUS: u16 = 0x0130;
    pub const AL_STATUS_CODE: u16 = 0x0134;
    pub const SII_CONFIG: u16 = 0x0500;
    pub const SII_CONTROL: u16 = 0x0502;
    pub const SII_ADDRESS: u16 = 0x0504;
    pub const SII_DATA: u16 = 0x0508;
    pub const FMMU0: u16 = 0x0600;
    pub const SM0: u16 = 0x0800;
    pub const DC_RECEIVE_TIME_PORT0: u16 = 0x0900;
    pub const DC_SYSTEM_TIME: u16 = 0x0910;
    pub const DC_RECEIVE_TIME_LOCAL: u16 = 0x0918;
    pub const DC_SYSTEM_TIME_OFFSET: u16 = 0x0920;
    pub const DC_TRANSMISSION_DELAY: u16 = 0x0928;
    pub const DC_SYSTEM_TIME_DIFFERENCE: u16 = 0x092C;
}

/// AL states.
pub mod state {
    pub const INIT: u8 = 0x01;
    pub const PRE_OP: u8 = 0x02;
    pub const BOOT: u8 = 0x03;
    pub const SAFE_OP: u8 = 0x04;
    pub const OP: u8 = 0x08;
    pub const ERROR_FLAG: u8 = 0x10;
}

/// AL status codes the emulation sets.
pub mod al_code {
    pub const NONE: u16 = 0x0000;
    pub const INVALID_STATE_CHANGE: u16 = 0x0011;
    pub const UNKNOWN_STATE: u16 = 0x0012;
    /// BOOT asked of a device whose EEPROM states no bootstrap mailbox.
    pub const BOOTSTRAP_NOT_SUPPORTED: u16 = 0x0013;
}

/// The memory an ESC holds: its registers (to 0x0FFF) and the 8 KB of process RAM register 0x0006
/// states, 0x1000 to 0x2FFF.
pub const MEMORY: usize = 0x3000;

/// One SubDevice: its ESC memory, its EEPROM (SII) words, its local clock's origin.
#[derive(Clone, Debug)]
pub struct Esc {
    pub mem: Vec<u8>,
    pub sii: Vec<u16>,
    /// Nanoseconds since the segment's start when the device's local clock read 0.
    pub clock_origin_ns: u64,
}

impl Esc {
    pub fn new(sii: Vec<u16>, clock_origin_ns: u64) -> Esc {
        let mut mem = vec![0u8; MEMORY];
        // An ESC of the generic kind: 8 FMMUs, 8 SyncManagers, 8 KB of RAM, port 0 and port 1
        // as MII, the features of a device with distributed clocks.
        mem[reg::TYPE as usize] = 0x11;
        mem[reg::FMMUS_SUPPORTED as usize] = 8;
        mem[reg::SMS_SUPPORTED as usize] = 8;
        mem[reg::RAM_SIZE as usize] = 8;
        mem[reg::PORT_DESCRIPTOR as usize] = 0x0F;
        mem[reg::FEATURES as usize..reg::FEATURES as usize + 2].copy_from_slice(&0x01FCu16.to_le_bytes());
        mem[reg::AL_STATUS as usize] = state::INIT;
        let mut esc = Esc { mem, sii, clock_origin_ns };
        esc.set_link(false);
        esc
    }

    /// The DL status: port 0's link up and its loop open, port 1's link up and open when a device
    /// follows on it, closed (looped back) otherwise.
    pub fn set_link(&mut self, next: bool) {
        // Bit 0 PDI operational, bit 4 port 0 link, bit 5 port 1 link, bit 8 port 0 loop closed,
        // bit 9 port 0 communication, bit 10 port 1 loop closed, bit 11 port 1 communication; ports
        // 2 and 3 closed and silent.
        let mut v: u16 = 0x0001 | 0x0010 | 0x0200;
        if next {
            v |= 0x0020 | 0x0800;
        } else {
            v |= 0x0400;
        }
        v |= 0x1000 | 0x4000;
        self.put16(reg::DL_STATUS, v);
    }

    pub fn get16(&self, at: u16) -> u16 {
        u16::from_le_bytes([self.mem[at as usize], self.mem[at as usize + 1]])
    }

    fn put16(&mut self, at: u16, v: u16) {
        self.mem[at as usize..at as usize + 2].copy_from_slice(&v.to_le_bytes());
    }

    pub fn station(&self) -> u16 {
        self.get16(reg::STATION_ADDRESS)
    }

    pub fn al_state(&self) -> u8 {
        self.mem[reg::AL_STATUS as usize] & 0x0F
    }

    /// Whether `[at, at + len)` lies in the memory.
    fn inside(at: u16, len: usize) -> bool {
        at as usize + len <= MEMORY
    }

    /// Bytes read at `at`, nothing past the memory (zeros there, as an ESC reads unmapped space).
    pub fn read(&self, at: u16, len: usize) -> Vec<u8> {
        let mut out = vec![0u8; len];
        for (i, b) in out.iter_mut().enumerate() {
            let a = at as usize + i;
            if a < MEMORY {
                *b = self.mem[a];
            }
        }
        out
    }

    /// A write at `at`, then the behaviour of the registers it touched. `now_ns` is the segment's
    /// clock, for the latch of the receive times.
    pub fn write(&mut self, at: u16, data: &[u8], now_ns: u64) {
        if !Esc::inside(at, data.len()) {
            return;
        }
        // Whether the write overlaps the register `r` of width `w`.
        let touched = |r: u16, w: usize| (at as usize) < r as usize + w && (r as usize) < at as usize + data.len();
        // The registers EtherCAT only reads keep their values: the type to the features, the
        // alias (loaded from the EEPROM), the DL and AL status and the AL status code, each
        // SyncManager's status and PDI control, the receive times (a write to port 0's latches
        // them, its data unkept), the processing unit's receive time and the system time
        // difference; the system time itself is compared, not stored.
        let keep: Vec<(u16, Vec<u8>)> = READ_ONLY.iter().filter(|(r, w)| touched(*r, *w)).map(|(r, w)| (*r, self.read(*r, *w))).collect();
        self.mem[at as usize..at as usize + data.len()].copy_from_slice(data);
        for (r, v) in keep {
            self.mem[r as usize..r as usize + v.len()].copy_from_slice(&v);
        }
        if touched(reg::DC_SYSTEM_TIME, 8) && at >= reg::DC_SYSTEM_TIME && at as usize + data.len() <= reg::DC_SYSTEM_TIME as usize + 8 {
            // A write of the system time (4 or 8 bytes) is the reference's time handed down: the
            // device compares it with its own and keeps the difference.
            let mut theirs = [0u8; 8];
            let from = (at - reg::DC_SYSTEM_TIME) as usize;
            theirs[from..from + data.len()].copy_from_slice(data);
            self.compare_system_time(u64::from_le_bytes(theirs), data.len() <= 4, now_ns);
        }
        if touched(reg::AL_CONTROL, 2) {
            self.al_control();
        }
        if touched(reg::SII_CONTROL, 2) {
            self.sii_command();
        }
        if touched(reg::DC_RECEIVE_TIME_PORT0, 4) {
            // A write to port 0's receive time latches every port's, and the local time of port 0.
            let local = self.local_time(now_ns);
            for p in 0..4u16 {
                let t = local.wrapping_add(u64::from(p) * 40) as u32;
                let a = reg::DC_RECEIVE_TIME_PORT0 + 4 * p;
                self.mem[a as usize..a as usize + 4].copy_from_slice(&t.to_le_bytes());
            }
            self.mem[reg::DC_RECEIVE_TIME_LOCAL as usize..reg::DC_RECEIVE_TIME_LOCAL as usize + 8].copy_from_slice(&local.to_le_bytes());
        }
    }

    /// The system time difference (0x092C) after the reference's time `theirs` was handed down:
    /// its own system time less theirs, as sign and magnitude (bit 31 set when its own is the
    /// smaller), the magnitude held to 31 bits; over the low 32 bits when only those were written.
    pub fn compare_system_time(&mut self, theirs: u64, low_32: bool, now_ns: u64) {
        let mine = self.system_time(now_ns);
        let (mine, theirs) = if low_32 { (mine & 0xFFFF_FFFF, theirs & 0xFFFF_FFFF) } else { (mine, theirs) };
        let v = if mine >= theirs { (mine - theirs).min(0x7FFF_FFFF) as u32 } else { 0x8000_0000 | (theirs - mine).min(0x7FFF_FFFF) as u32 };
        let a = reg::DC_SYSTEM_TIME_DIFFERENCE as usize;
        self.mem[a..a + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// Whether the configured-address commands match the alias too (DL control bit 24).
    pub fn alias_enabled(&self) -> bool {
        self.mem[reg::DL_CONTROL_ALIAS as usize] & 1 != 0
    }

    /// The device's local time: the segment's clock less its origin.
    pub fn local_time(&self, now_ns: u64) -> u64 {
        now_ns.saturating_sub(self.clock_origin_ns)
    }

    /// The system time it reads at `now_ns`: its local time plus the offset the master wrote.
    pub fn system_time(&self, now_ns: u64) -> u64 {
        let offset = u64::from_le_bytes(self.read(reg::DC_SYSTEM_TIME_OFFSET, 8).try_into().unwrap_or([0; 8]));
        self.local_time(now_ns).wrapping_add(offset)
    }

    /// A requested AL state: a transition the state machine allows is taken at once; another is
    /// refused with the error flag and its code, until the master acknowledges it.
    fn al_control(&mut self) {
        let control = self.get16(reg::AL_CONTROL);
        let requested = (control & 0x0F) as u8;
        let ack = control & 0x10 != 0;
        let current = self.al_state();
        let error = self.mem[reg::AL_STATUS as usize] & state::ERROR_FLAG != 0;
        if error && !ack && requested != state::INIT {
            return;
        }
        let allowed = matches!(
            (current, requested),
            (_, state::INIT)
                | (state::INIT, state::PRE_OP | state::BOOT)
                | (state::PRE_OP, state::PRE_OP | state::SAFE_OP)
                | (state::SAFE_OP, state::PRE_OP | state::SAFE_OP | state::OP)
                | (state::OP, state::PRE_OP | state::SAFE_OP | state::OP)
                | (state::BOOT, state::BOOT)
        );
        let known = matches!(requested, state::INIT | state::PRE_OP | state::BOOT | state::SAFE_OP | state::OP);
        if requested == state::BOOT && current == state::INIT {
            // The EEPROM states no bootstrap mailbox: BOOT is never taken.
            self.mem[reg::AL_STATUS as usize] = current | state::ERROR_FLAG;
            self.put16(reg::AL_STATUS_CODE, al_code::BOOTSTRAP_NOT_SUPPORTED);
            return;
        }
        if known && allowed {
            self.mem[reg::AL_STATUS as usize] = requested;
            self.put16(reg::AL_STATUS_CODE, al_code::NONE);
        } else {
            self.mem[reg::AL_STATUS as usize] = current | state::ERROR_FLAG;
            self.put16(reg::AL_STATUS_CODE, if known { al_code::INVALID_STATE_CHANGE } else { al_code::UNKNOWN_STATE });
        }
    }

    /// An SII command: a read loads the data register from the EEPROM (8 bytes, as an ESC that
    /// reads 64 bits at a time), done at once (never busy).
    fn sii_command(&mut self) {
        let control = self.get16(reg::SII_CONTROL);
        let command = (control >> 8) & 0x07;
        if command == 0x01 {
            let word = u32::from_le_bytes(self.read(reg::SII_ADDRESS, 4).try_into().unwrap_or([0; 4])) as usize;
            for i in 0..4 {
                let w = self.sii.get(word + i).copied().unwrap_or(0xFFFF);
                let a = reg::SII_DATA as usize + 2 * i;
                self.mem[a..a + 2].copy_from_slice(&w.to_le_bytes());
            }
        }
        // Done: the command bits cleared, not busy, 8-byte reads (bit 6), no error.
        self.put16(reg::SII_CONTROL, 0x0040);
    }

    /// The FMMUs active, as their registers hold them.
    pub fn fmmus(&self) -> Vec<Fmmu> {
        (0..8u16)
            .filter_map(|i| {
                let b = self.read(reg::FMMU0 + 16 * i, 16);
                if b[12] & 1 == 0 {
                    return None;
                }
                Some(Fmmu {
                    logical: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                    len: u16::from_le_bytes([b[4], b[5]]),
                    start_bit: b[6] & 7,
                    end_bit: b[7] & 7,
                    physical: u16::from_le_bytes([b[8], b[9]]),
                    physical_bit: b[10] & 7,
                    read: b[11] & 1 != 0,
                    write: b[11] & 2 != 0,
                })
            })
            .collect()
    }
}

/// The registers EtherCAT only reads, each (address, width).
const READ_ONLY: [(u16, usize); 24] = [
    (reg::TYPE, 10),
    (reg::STATION_ALIAS, 2),
    (reg::DL_STATUS, 2),
    (reg::AL_STATUS, 2),
    (reg::AL_STATUS_CODE, 2),
    (0x0805, 1),
    (0x080D, 1),
    (0x0815, 1),
    (0x081D, 1),
    (0x0825, 1),
    (0x082D, 1),
    (0x0835, 1),
    (0x083D, 1),
    (0x0807, 1),
    (0x080F, 1),
    (0x0817, 1),
    (0x081F, 1),
    (0x0827, 1),
    (0x082F, 1),
    (0x0837, 1),
    (0x083F, 1),
    (reg::DC_RECEIVE_TIME_PORT0, 16),
    (reg::DC_RECEIVE_TIME_LOCAL, 8),
    (reg::DC_SYSTEM_TIME_DIFFERENCE, 4),
];

/// One FMMU: a span of logical bits mapped onto physical bits from a start bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fmmu {
    pub logical: u32,
    /// Bytes the logical span touches, its first and last partly when the bits say so.
    pub len: u16,
    pub start_bit: u8,
    pub end_bit: u8,
    pub physical: u16,
    pub physical_bit: u8,
    pub read: bool,
    pub write: bool,
}

/// One datagram of a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Datagram {
    pub cmd: u8,
    pub idx: u8,
    pub address: u32,
    pub circulating: bool,
    pub more: bool,
    pub irq: u16,
    pub data: Vec<u8>,
    pub wkc: u16,
}

impl Datagram {
    fn adp(&self) -> u16 {
        self.address as u16
    }

    fn ado(&self) -> u16 {
        (self.address >> 16) as u16
    }

    fn set_adp(&mut self, v: u16) {
        self.address = (self.address & 0xFFFF_0000) | u32::from(v);
    }
}

/// The datagrams of an EtherCAT frame's payload (after the Ethernet header): its 2-byte header
/// (length, type 1), then the datagrams. `None` when it is not a frame of commands or runs short.
pub fn parse(payload: &[u8]) -> Option<Vec<Datagram>> {
    if payload.len() < 2 {
        return None;
    }
    let header = u16::from_le_bytes([payload[0], payload[1]]);
    let len = (header & 0x07FF) as usize;
    if header >> 12 != 1 || 2 + len > payload.len() {
        return None;
    }
    let body = &payload[2..2 + len];
    let mut out = Vec::new();
    let mut at = 0;
    loop {
        if at + 10 > body.len() {
            return None;
        }
        let cmd = body[at];
        let idx = body[at + 1];
        let address = u32::from_le_bytes([body[at + 2], body[at + 3], body[at + 4], body[at + 5]]);
        let lf = u16::from_le_bytes([body[at + 6], body[at + 7]]);
        let dlen = (lf & 0x07FF) as usize;
        let irq = u16::from_le_bytes([body[at + 8], body[at + 9]]);
        if at + 10 + dlen + 2 > body.len() {
            return None;
        }
        let data = body[at + 10..at + 10 + dlen].to_vec();
        let wkc = u16::from_le_bytes([body[at + 10 + dlen], body[at + 11 + dlen]]);
        let more = lf & 0x8000 != 0;
        out.push(Datagram { cmd, idx, address, circulating: lf & 0x4000 != 0, more, irq, data, wkc });
        at += 12 + dlen;
        if !more {
            break;
        }
    }
    Some(out)
}

/// A frame's payload written back: the header, the datagrams, padding as the frame had it.
pub fn encode(datagrams: &[Datagram], original_len: usize) -> Vec<u8> {
    let mut body = Vec::new();
    for d in datagrams {
        body.push(d.cmd);
        body.push(d.idx);
        body.extend_from_slice(&d.address.to_le_bytes());
        let mut lf = d.data.len() as u16 & 0x07FF;
        if d.circulating {
            lf |= 0x4000;
        }
        if d.more {
            lf |= 0x8000;
        }
        body.extend_from_slice(&lf.to_le_bytes());
        body.extend_from_slice(&d.irq.to_le_bytes());
        body.extend_from_slice(&d.data);
        body.extend_from_slice(&d.wkc.to_le_bytes());
    }
    let header = (body.len() as u16 & 0x07FF) | (1 << 12);
    let mut out = header.to_le_bytes().to_vec();
    out.extend(body);
    if out.len() < original_len {
        out.resize(original_len, 0);
    }
    out
}

/// The frame's datagrams through the segment, each device in order as the frame passes it.
pub fn process(segment: &mut [Esc], datagrams: &mut [Datagram], now_ns: u64) {
    for d in datagrams.iter_mut() {
        for esc in segment.iter_mut() {
            pass(esc, d, now_ns);
        }
    }
}

fn pass(esc: &mut Esc, d: &mut Datagram, now_ns: u64) {
    use cmd::*;
    let len = d.data.len();
    match d.cmd {
        APRD | APWR | APRW | ARMW => {
            let addressed = d.adp() == 0;
            if addressed || d.cmd == ARMW {
                physical(esc, d, addressed, now_ns);
            }
            d.set_adp(d.adp().wrapping_add(1));
        }
        FPRD | FPWR | FPRW | FRMW => {
            let addressed = d.adp() == esc.station() || (esc.alias_enabled() && d.adp() == esc.get16(reg::STATION_ALIAS));
            if addressed || d.cmd == FRMW {
                physical(esc, d, addressed, now_ns);
            }
        }
        // A broadcast's position field is not used for addressing, and every device still adds
        // 1 to it on the way (the ESC datasheet's addressing table).
        BRD => {
            let local = local_read(esc, d.ado(), len, now_ns);
            for (x, y) in d.data.iter_mut().zip(local) {
                *x |= y;
            }
            d.wkc = d.wkc.wrapping_add(1);
            d.set_adp(d.adp().wrapping_add(1));
        }
        BWR => {
            esc.write(d.ado(), &d.data, now_ns);
            d.wkc = d.wkc.wrapping_add(1);
            d.set_adp(d.adp().wrapping_add(1));
        }
        BRW => {
            let local = local_read(esc, d.ado(), len, now_ns);
            esc.write(d.ado(), &d.data, now_ns);
            for (x, y) in d.data.iter_mut().zip(local) {
                *x |= y;
            }
            d.wkc = d.wkc.wrapping_add(3);
            d.set_adp(d.adp().wrapping_add(1));
        }
        LRD | LWR | LRW => logical(esc, d, now_ns),
        _ => {}
    }
}

/// A read of `len` bytes at `at`, the system time read live where it is asked.
fn local_read(esc: &Esc, at: u16, len: usize, now_ns: u64) -> Vec<u8> {
    let mut out = esc.read(at, len);
    let st = esc.system_time(now_ns).to_le_bytes();
    for (i, b) in out.iter_mut().enumerate() {
        let a = at as usize + i;
        if (reg::DC_SYSTEM_TIME as usize..reg::DC_SYSTEM_TIME as usize + 8).contains(&a) {
            *b = st[a - reg::DC_SYSTEM_TIME as usize];
        }
    }
    out
}

fn physical(esc: &mut Esc, d: &mut Datagram, addressed: bool, now_ns: u64) {
    use cmd::*;
    let at = d.ado();
    match d.cmd {
        APRD | FPRD => {
            d.data = local_read(esc, at, d.data.len(), now_ns);
            d.wkc = d.wkc.wrapping_add(1);
        }
        APWR | FPWR => {
            esc.write(at, &d.data, now_ns);
            d.wkc = d.wkc.wrapping_add(1);
        }
        APRW | FPRW => {
            let local = local_read(esc, at, d.data.len(), now_ns);
            esc.write(at, &d.data, now_ns);
            d.data = local;
            d.wkc = d.wkc.wrapping_add(3);
        }
        // Read by the one addressed, written by every other: the reference clock's time handed to
        // the devices after it.
        ARMW | FRMW => {
            if addressed {
                d.data = local_read(esc, at, d.data.len(), now_ns);
            } else {
                esc.write(at, &d.data, now_ns);
            }
            d.wkc = d.wkc.wrapping_add(1);
        }
        _ => {}
    }
}

/// A logical command through the device's FMMUs, bit by bit: each mapped bit inside the datagram's
/// span is read into the datagram (LRD, LRW) or written to the device (LWR, LRW). A mapping that
/// both reads and writes writes what the datagram brought to the device and reads what the device
/// held before. The counter counts a device 1 for a read, 1 for a write, 2 for an LRW's write.
fn logical(esc: &mut Esc, d: &mut Datagram, now_ns: u64) {
    use cmd::*;
    let bit = |bytes: &[u8], i: u64| bytes[(i / 8) as usize] >> (i % 8) & 1;
    let set = |bytes: &mut [u8], i: u64, v: u8| {
        let (byte, mask) = ((i / 8) as usize, 1u8 << (i % 8));
        bytes[byte] = if v == 1 { bytes[byte] | mask } else { bytes[byte] & !mask };
    };
    let first = u64::from(d.address) * 8;
    let last = first + d.data.len() as u64 * 8;
    let incoming = d.data.clone();
    let (mut read, mut wrote) = (false, false);
    for f in esc.fmmus() {
        if f.len == 0 {
            continue;
        }
        let lo = u64::from(f.logical) * 8 + u64::from(f.start_bit);
        let hi = (u64::from(f.logical) + u64::from(f.len) - 1) * 8 + u64::from(f.end_bit) + 1;
        let (from, to) = (lo.max(first), hi.min(last));
        let reads = f.read && matches!(d.cmd, LRD | LRW);
        let writes = f.write && matches!(d.cmd, LWR | LRW);
        if from >= to || !(reads || writes) {
            continue;
        }
        let p0 = u64::from(f.physical) * 8 + u64::from(f.physical_bit) + (from - lo);
        let n = to - from;
        let (byte_lo, byte_hi) = (p0 / 8, (p0 + n - 1) / 8 + 1);
        if byte_hi > 0x1_0000 {
            continue;
        }
        let held = esc.read(byte_lo as u16, (byte_hi - byte_lo) as usize);
        let mut bytes = held.clone();
        for k in 0..n {
            let (lb, pb) = (from + k - first, p0 + k - byte_lo * 8);
            if reads {
                set(&mut d.data, lb, bit(&held, pb));
            }
            if writes {
                set(&mut bytes, pb, bit(&incoming, lb));
            }
        }
        read |= reads;
        if writes {
            esc.write(byte_lo as u16, &bytes, now_ns);
            wrote = true;
        }
    }
    if read {
        d.wkc = d.wkc.wrapping_add(1);
    }
    if wrote {
        d.wkc = d.wkc.wrapping_add(if d.cmd == LRW { 2 } else { 1 });
    }
}

/// One process-data entry: an object's index, subindex and size in bits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdoEntry {
    pub index: u16,
    pub subindex: u8,
    pub bits: u8,
}

/// What a SubDevice says of itself in its EEPROM.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub vendor_id: u32,
    pub product_code: u32,
    pub revision: u32,
    pub serial: u32,
    pub name: String,
    pub alias: u16,
}

/// Where the process data sits in the device's RAM: outputs at 0x1000, inputs after them.
pub const OUTPUTS_AT: u16 = 0x1000;

/// The inputs' place: after the outputs, on an even address.
pub fn inputs_at(output_bytes: usize) -> u16 {
    OUTPUTS_AT + ((output_bytes as u16 + 1) & !1).max(2)
}

/// The SII image: the header words (identity, no mailbox), then the categories: strings, general,
/// FMMU usage, SyncManagers (SM2 outputs, SM3 inputs), the TxPDO (inputs) and the RxPDO (outputs),
/// the end. A device with no mailbox: its PDOs are read from here, not by CoE.
pub fn sii_image(id: &Identity, outputs: &[PdoEntry], inputs: &[PdoEntry]) -> Vec<u16> {
    let mut w = vec![0u16; 0x40];
    w[0x00] = 0x0C80;
    w[0x04] = id.alias;
    let crc = sii_crc(&w[..7]);
    w[0x07] = u16::from(crc);
    let put32 = |w: &mut Vec<u16>, at: usize, v: u32| {
        w[at] = v as u16;
        w[at + 1] = (v >> 16) as u16;
    };
    put32(&mut w, 0x08, id.vendor_id);
    put32(&mut w, 0x0A, id.product_code);
    put32(&mut w, 0x0C, id.revision);
    put32(&mut w, 0x0E, id.serial);
    // No mailbox: offsets and sizes 0, protocols none; the size (word 0x3E) is set at the end.
    w[0x3F] = 0x0001;
    let bytes_of = |entries: &[PdoEntry]| entries.iter().map(|e| e.bits as usize).sum::<usize>().div_ceil(8);
    let (out_len, in_len) = (bytes_of(outputs), bytes_of(inputs));
    let cat = |w: &mut Vec<u16>, kind: u16, data: Vec<u8>| {
        let mut data = data;
        if data.len() % 2 == 1 {
            data.push(0);
        }
        w.push(kind);
        w.push((data.len() / 2) as u16);
        for c in data.chunks(2) {
            w.push(u16::from_le_bytes([c[0], c[1]]));
        }
    };
    // Strings: 1 the device's name.
    let mut strings = vec![1u8, id.name.len() as u8];
    strings.extend_from_slice(id.name.as_bytes());
    cat(&mut w, 10, strings);
    // General: group and image none; the order number and the name both string 1 (a master names
    // a device by its order number); physical layer port 0 and 1 MII.
    let mut general = vec![0u8; 32];
    general[2] = 1;
    general[3] = 1;
    general[0x10] = 0x11;
    cat(&mut w, 30, general);
    // FMMU usage: 0 outputs, 1 inputs.
    cat(&mut w, 40, vec![1, 2]);
    // SyncManagers: SM0 and SM1 unused (no mailbox), SM2 outputs, SM3 inputs.
    let sm = |start: u16, len: usize, control: u8, enable: u8, kind: u8| {
        let mut b = start.to_le_bytes().to_vec();
        b.extend_from_slice(&(len as u16).to_le_bytes());
        b.extend_from_slice(&[control, 0, enable, kind]);
        b
    };
    let mut sms = Vec::new();
    sms.extend(sm(0, 0, 0, 0, 0));
    sms.extend(sm(0, 0, 0, 0, 0));
    sms.extend(sm(OUTPUTS_AT, out_len, 0x64, if out_len > 0 { 1 } else { 0 }, 3));
    sms.extend(sm(inputs_at(out_len), in_len, 0x20, if in_len > 0 { 1 } else { 0 }, 4));
    cat(&mut w, 41, sms);
    let pdo = |index: u16, sm: u8, entries: &[PdoEntry]| {
        let mut b = index.to_le_bytes().to_vec();
        b.extend_from_slice(&[entries.len() as u8, sm, 0, 0]);
        b.extend_from_slice(&0u16.to_le_bytes());
        for e in entries {
            b.extend_from_slice(&e.index.to_le_bytes());
            b.extend_from_slice(&[e.subindex, 0, 0, e.bits]);
            b.extend_from_slice(&0u16.to_le_bytes());
        }
        b
    };
    if !inputs.is_empty() {
        cat(&mut w, 50, pdo(0x1A00, 3, inputs));
    }
    if !outputs.is_empty() {
        cat(&mut w, 51, pdo(0x1600, 2, outputs));
    }
    w.push(0xFFFF);
    // The EEPROM's size, in Kbit less 1: the smallest power of two from 16 Kbit that holds it.
    let kbit = (w.len() * 16).div_ceil(1024).next_power_of_two().max(16);
    w[0x3E] = (kbit - 1) as u16;
    w
}

/// The SII header's checksum: CRC-8 (polynomial 0x07, start 0xFF) over its first 14 bytes.
pub fn sii_crc(words: &[u16]) -> u8 {
    let mut crc: u8 = 0xFF;
    for w in words {
        for b in w.to_le_bytes() {
            crc ^= b;
            for _ in 0..8 {
                crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
            }
        }
    }
    crc
}

