//! The GENIBELT message catalog — every message the host interface
//! defines, typed.
//!
//! Authority: `docs/vendor/genibelt-protocol.txt` (machine-local, see
//! `docs/vendor/README.md`). Section numbers in the comments below refer
//! to it, and every `MSG LENGTH` here is the value the spec declares —
//! `sizes_match_the_specification` pins the whole table in one test, so
//! a typo in a layout fails loudly rather than desynchronizing a plant.
//!
//! Two message families (§10.1, §10.2):
//!
//! - **OAMTORTX** (50000…) — External Unit → Master Sort Control. What
//!   veredas SENDS: destinations, commands, lamp and bin state.
//! - **RTXTOOAM** (51000…) — Master Sort Control → External Unit. What
//!   veredas RECEIVES: scanner requests, outcomes, statuses.
//!
//! Plus the small side families (40000 supervisor, 52002 generic
//! exchange, 55000 induction-line item-lost, 60000 ISPC diagnostic).
//!
//! Everything is length-checked on the way in: a short or oversized body
//! is a typed error, never a slice panic — these bytes arrive from a
//! LAN, and a malformed frame must not take the source down.

use crate::telegram::Endian;

/// Message identifiers (§10.1, §10.2, §10.16–10.21).
pub mod ids {
    // External Unit → MSC (§10.1)
    pub const OAMTORTX_SYSTEM: u16 = 50000;
    pub const OAMTORTX_COMMANDS: u16 = 50001;
    pub const OAMTORTX_DESTINATION: u16 = 50002;
    pub const OAMTORTX_LAMP: u16 = 50003;
    pub const OAMTORTX_CODINGANSWER: u16 = 50004;
    pub const OAMTORTX_EUBINSTATE: u16 = 50005;
    pub const OAMTORTX_DEVICES: u16 = 50006;
    pub const OAMTORTX_EUIBINLCDDISPLAY: u16 = 50007;
    pub const OAMTORTX_OUTPUTSIGNALS: u16 = 50008;
    pub const OAMTORTX_BINSTATUS_ALL: u16 = 50009;
    pub const OAMTORTX_CELL_RELOAD: u16 = 50010;
    pub const OAMTORTX_ILBELT_MAINTENANCE: u16 = 50011;
    pub const OAMTORTX_COMMANDS_EX: u16 = 50012;
    pub const OAMTORTX_CELL_RELOAD_EX: u16 = 50013;

    // MSC → External Unit (§10.2)
    pub const RTXTOOAM_OUTCOME: u16 = 51000;
    pub const RTXTOOAM_SORTERSTATUS: u16 = 51001;
    pub const RTXTOOAM_BINPUSHBUTTONS: u16 = 51002;
    pub const RTXTOOAM_BINSTATUS: u16 = 51003;
    pub const RTXTOOAM_SCANNER: u16 = 51004;
    pub const RTXTOOAM_CODINGREQUEST: u16 = 51005;
    pub const RTXTOOAM_CODINGRESULT: u16 = 51006;
    pub const RTXTOOAM_EVENT_SORTERLAPS: u16 = 51007;
    pub const RTXTOOAM_EVENT_UPSSTATUS: u16 = 51008;
    pub const RTXTOOAM_MSCDEVICES: u16 = 51009;
    pub const RTXTOOAM_PARCEL_LOADED: u16 = 51010;
    pub const RTXTOOAM_PARCEL_RECIRCULATED: u16 = 51011;
    pub const RTXTOOAM_INVENTORY_BEGIN: u16 = 51012;
    pub const RTXTOOAM_INVENTORY_END: u16 = 51013;
    pub const RTXTOOAM_SORTER_EMPTY: u16 = 51014;
    pub const RTXTOOAM_PARCEL_ON_CTRLZONE: u16 = 51015;
    pub const RTXTOOAM_SORTERSTATUS_EX: u16 = 51016;

    // Side families
    pub const CSPCTOEUI_LAMPTEST: u16 = 40000;
    pub const GENERIC_DATAEXCHANGE: u16 = 52002;
    pub const CONVSYSTOOAM_ITEMLOST: u16 = 55000;
    pub const ISPCTORTX_DIAGNOSTIC: u16 = 60000;
}

/// §3.3: "The maximum size of the DATA SEGMENT is equal to 512 bytes."
pub const MAX_DATA_SEGMENT: usize = 512;
/// §3.3: "The maximum size of the PROTOCOL is equal to 5 Kbytes."
pub const MAX_PROTOCOL: usize = 5 * 1024;

/// The two fixed-size text blocks that carry scanner and induction data.
pub const SZ_DATA_LEN: usize = 240;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MsgError {
    /// The body is not the length the spec declares for this id.
    WrongLength {
        msg_id: u16,
        expected: usize,
        actual: usize,
    },
    /// A body longer than the protocol allows (§3.3).
    TooLong { msg_id: u16, len: usize },
}

impl std::fmt::Display for MsgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MsgError::WrongLength {
                msg_id,
                expected,
                actual,
            } => write!(
                f,
                "message {msg_id} should carry {expected} bytes, got {actual}"
            ),
            MsgError::TooLong { msg_id, len } => write!(
                f,
                "message {msg_id} carries {len} bytes — over the {MAX_DATA_SEGMENT}-byte segment limit"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Checked little readers/writers
// ---------------------------------------------------------------------------

/// A cursor over a message body. Every read is bounds-checked — these
/// bytes come off a socket.
struct Reader<'a> {
    buf: &'a [u8],
    at: usize,
    endian: Endian,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8], endian: Endian) -> Reader<'a> {
        Reader { buf, at: 0, endian }
    }

    fn u8(&mut self) -> u8 {
        let v = self.buf.get(self.at).copied().unwrap_or(0);
        self.at += 1;
        v
    }

    fn i8(&mut self) -> i8 {
        self.u8() as i8
    }

    fn u16(&mut self) -> u16 {
        let bytes = [
            self.buf.get(self.at).copied().unwrap_or(0),
            self.buf.get(self.at + 1).copied().unwrap_or(0),
        ];
        self.at += 2;
        match self.endian {
            Endian::Big => u16::from_be_bytes(bytes),
            Endian::Little => u16::from_le_bytes(bytes),
        }
    }

    fn u32(&mut self) -> u32 {
        let mut bytes = [0u8; 4];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = self.buf.get(self.at + i).copied().unwrap_or(0);
        }
        self.at += 4;
        match self.endian {
            Endian::Big => u32::from_be_bytes(bytes),
            Endian::Little => u32::from_le_bytes(bytes),
        }
    }

    /// Windows `BOOL` — four bytes on the wire.
    fn bool32(&mut self) -> bool {
        self.u32() != 0
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        let end = (self.at + n).min(self.buf.len());
        let mut out = self.buf.get(self.at..end).unwrap_or(&[]).to_vec();
        out.resize(n, 0);
        self.at += n;
        out
    }

    fn array_u16(&mut self, n: usize) -> Vec<u16> {
        (0..n).map(|_| self.u16()).collect()
    }

    fn array_i8(&mut self, n: usize) -> Vec<i8> {
        (0..n).map(|_| self.i8()).collect()
    }
}

struct Writer {
    out: Vec<u8>,
    endian: Endian,
}

impl Writer {
    fn new(endian: Endian) -> Writer {
        Writer {
            out: Vec::new(),
            endian,
        }
    }

    fn u8(&mut self, v: u8) -> &mut Self {
        self.out.push(v);
        self
    }

    fn i8(&mut self, v: i8) -> &mut Self {
        self.out.push(v as u8);
        self
    }

    fn u16(&mut self, v: u16) -> &mut Self {
        match self.endian {
            Endian::Big => self.out.extend_from_slice(&v.to_be_bytes()),
            Endian::Little => self.out.extend_from_slice(&v.to_le_bytes()),
        }
        self
    }

    fn u32(&mut self, v: u32) -> &mut Self {
        match self.endian {
            Endian::Big => self.out.extend_from_slice(&v.to_be_bytes()),
            Endian::Little => self.out.extend_from_slice(&v.to_le_bytes()),
        }
        self
    }

    fn bool32(&mut self, v: bool) -> &mut Self {
        self.u32(if v { 1 } else { 0 })
    }

    /// A fixed-width block: truncated or zero-padded to exactly `n`.
    fn block(&mut self, data: &[u8], n: usize) -> &mut Self {
        let take = data.len().min(n);
        self.out.extend_from_slice(&data[..take]);
        self.out.resize(self.out.len() + (n - take), 0);
        self
    }

    fn array_u16(&mut self, values: &[u16], n: usize) -> &mut Self {
        for i in 0..n {
            let v = values.get(i).copied().unwrap_or(0);
            self.u16(v);
        }
        self
    }

    fn array_i8(&mut self, values: &[i8], n: usize) -> &mut Self {
        for i in 0..n {
            let v = values.get(i).copied().unwrap_or(0);
            self.i8(v);
        }
        self
    }

    /// Takes the bytes out, so the chained `&mut Self` builders can end
    /// in a value.
    fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }
}

/// Check a body's length before decoding it.
fn expect_len(msg_id: u16, data: &[u8], expected: usize) -> Result<(), MsgError> {
    if data.len() == expected {
        Ok(())
    } else {
        Err(MsgError::WrongLength {
            msg_id,
            expected,
            actual: data.len(),
        })
    }
}

/// An ASCIIZ field as text (stops at the first NUL, §4.1.1.7 note).
pub fn asciiz(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end])
        .trim_end()
        .to_string()
}

/// A UNICODE (UTF-16) field as text (§4.1.1.6: "One UNICODE char is made
/// by 2 byte").
pub fn utf16z(bytes: &[u8], endian: Endian) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|p| match endian {
            Endian::Big => u16::from_be_bytes([p[0], p[1]]),
            Endian::Little => u16::from_le_bytes([p[0], p[1]]),
        })
        .take_while(|u| *u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

/// Encode text into a UTF-16 block of `bytes` bytes.
pub fn utf16_block(text: &str, bytes: usize, endian: Endian) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes);
    for unit in text.encode_utf16() {
        if out.len() + 2 > bytes {
            break;
        }
        match endian {
            Endian::Big => out.extend_from_slice(&unit.to_be_bytes()),
            Endian::Little => out.extend_from_slice(&unit.to_le_bytes()),
        }
    }
    out.resize(bytes, 0);
    out
}

// ---------------------------------------------------------------------------
// External Unit → MSC (OAMTORTX)
// ---------------------------------------------------------------------------

/// §4.1.1.1 — system request (shutdown, UPS activated). 2 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct System {
    pub system: u16,
}

/// §4.1.1.2 — MSC/ISPC start, stop, reset. 10 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Commands {
    pub command: u8,
    pub operator_id: u8,
    pub param0: u8,
    pub param1: u8,
    pub param2: u16,
    pub param3: u32,
}

/// §4.1.1.3 — the extended form, with diagnostic attributes. 15 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CommandsEx {
    pub command: u8,
    pub operator_id: u8,
    pub param0: u8,
    pub param1: u8,
    pub param2: u16,
    pub param3: u32,
    pub class_id: u8,
    pub element_id: u16,
    pub status_alarm_warning: u8,
    pub bit_number: u8,
}

/// §4.1.2.1 — **the destination assignment**: the answer to a scanner
/// request, and the message this whole chain exists to send. 46 bytes,
/// MANDATORY per the spec.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Destination {
    pub object_id: u32,
    /// "It must always contain a valid destination number."
    pub dest: u16,
    pub cell: u16,
    /// Zero except for MSC-crash recovery ("cell reload").
    pub used_cells: u8,
    /// See [`dest_flag`] — 3 = ASSIGNED is the normal flow, 1 =
    /// FORCE_REASSIGNEMENT is "ask me again next lap".
    pub flag: u8,
    pub scn_id: u8,
    pub operator_id: u8,
    /// Unloading profile; 0 = default.
    pub ramp_id: u8,
    /// Unload-point offset, NIF (50 mm) based.
    pub dest_unload_offset: i8,
    pub oam_reject_anomaly: u8,
    pub num_alternative_dest: u8,
    pub alternative_dest: Vec<u16>,
    pub alternative_dest_unload_offset: Vec<i8>,
}

impl Default for Destination {
    /// The alternate arrays are FIXED at 10 on the wire (§4.1.2.1), so
    /// the default carries ten zeros rather than empty vectors — a
    /// default that decodes differently from how it encodes would be a
    /// trap for every caller that builds one field at a time.
    fn default() -> Destination {
        Destination {
            object_id: 0,
            dest: 0,
            cell: 0,
            used_cells: 0,
            flag: 0,
            scn_id: 0,
            operator_id: 0,
            ramp_id: 0,
            dest_unload_offset: 0,
            oam_reject_anomaly: 0,
            num_alternative_dest: 0,
            alternative_dest: vec![0; 10],
            alternative_dest_unload_offset: vec![0; 10],
        }
    }
}

impl Destination {
    /// The normal answer: sort this parcel to `dest`.
    pub fn assigned(object_id: u32, cell: u16, dest: u16) -> Destination {
        Destination {
            object_id,
            cell,
            dest,
            flag: dest_flag::ASSIGNED,
            ..Default::default()
        }
    }

    /// "Ask me again next lap" — the honest answer when no decision is
    /// available (§4.1.2.1: the External Unit does not yet have the
    /// information to sort this item). `wDest` is not checked by the MSC
    /// under this flag.
    pub fn force_reassignment(object_id: u32, cell: u16) -> Destination {
        Destination {
            object_id,
            cell,
            dest: 0,
            flag: dest_flag::FORCE_REASSIGNEMENT,
            ..Default::default()
        }
    }
}

/// §10.6 — `byFlag` values of [`Destination`].
pub mod dest_flag {
    pub const SCANNER_ONCEAGAIN: u8 = 0;
    /// Ask again on the next re-destination point — the honest answer
    /// when no decision is available yet.
    pub const FORCE_REASSIGNEMENT: u8 = 1;
    /// "Park": the sorter stops asking until a destination is pushed.
    pub const WAIT_FOR_DESTINATION: u8 = 2;
    /// Normal flow.
    pub const ASSIGNED: u8 = 3;
    pub const ASSIGNED_ORDER_BY_DEST: u8 = 4;
    pub const PRIMARY_PLUS_ALTERNATE: u8 = 5;
    pub const ALTERNATE_IF_NO_PRIMARY: u8 = 6;

    pub fn name(v: u8) -> &'static str {
        match v {
            SCANNER_ONCEAGAIN => "scanner once again",
            FORCE_REASSIGNEMENT => "force reassignment",
            WAIT_FOR_DESTINATION => "wait for destination",
            ASSIGNED => "assigned",
            ASSIGNED_ORDER_BY_DEST => "assigned, ordered by destination",
            PRIMARY_PLUS_ALTERNATE => "primary plus alternate",
            ALTERNATE_IF_NO_PRIMARY => "alternate if no primary",
            _ => "unknown",
        }
    }
}

/// §4.1.1.4 — bin lamp control. 14 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Lamp {
    pub device_id: u16,
    pub element: u16,
    pub command: u8,
    /// TRUE = the sorter never unloads into this destination.
    pub inhibit_unload: bool,
    pub lamp_type: u8,
    pub time_on_ms: u16,
    pub time_off_ms: u16,
}

/// §4.1.1.5 — the External Unit's own view of a bin's logical state.
/// 6 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EuBinState {
    pub dest: u16,
    pub state: u32,
}

/// §4.1.2.2 — answer to a sorter-device request. 12 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Devices {
    pub device_type: u32,
    pub device_id: u8,
    pub cell: u16,
    pub flag: u8,
    pub object_id: u32,
}

/// §4.1.1.6 — 40 UNICODE characters for a bin's LCD. 82 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BinLcdDisplay {
    pub dest: u16,
    pub text: String,
}

/// §4.1.1.7 — MSC output signal control. 34 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OutputSignals {
    pub class_id: u8,
    pub device_id: u16,
    /// ASCIIZ, 26 bytes on the wire (25 usable).
    pub output_name: String,
    pub output_state: u8,
    pub time_on_ms: u16,
    pub time_off_ms: u16,
}

/// §4.1.2.3 — cell reload. 8 + X bytes (custom data is plant-specific).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CellReload {
    pub cell: u16,
    pub used_cells: u8,
    pub ispc_id: u8,
    pub object_id: u32,
    pub custom_data: Vec<u8>,
}

/// §4.1.2.4 — the extended cell reload. 249 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellReloadEx {
    pub cell: u16,
    pub used_cells: u8,
    pub ispc_id: u8,
    pub rescan_required: u8,
    pub object_id: u32,
    /// 240 bytes — the same `STRUCT_ISPC_MSC_DATA` shape the scanner
    /// request carries.
    pub user_data: Vec<u8>,
}

impl Default for CellReloadEx {
    fn default() -> CellReloadEx {
        CellReloadEx {
            cell: 0,
            used_cells: 0,
            ispc_id: 0,
            rescan_required: 0,
            object_id: 0,
            user_data: vec![0; SZ_DATA_LEN],
        }
    }
}

// Fixed-width blocks and their defaults
// --------------------------------------
// Several messages carry byte blocks whose width is FIXED on the wire
// (szMSCData and szUserData are 240 bytes, szMsgToShow 128,
// szGenericData 498, the alibi arrays 35 and 42). A derived `Default`
// would give empty vectors, so a value written as
// `Struct { one_field, ..Default::default() }` would not equal itself
// after an encode/decode round trip — a trap for every caller, and a lie
// in every test. Each default below carries the real widths, so a
// partially-built message is still wire-shaped.

/// §5.2.1.1 — the manual-coding answer. 375 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingAnswer {
    pub operator_id: u8,
    pub timeout_10ms: u16,
    pub user_data: Vec<u8>,
    pub item_expected: bool,
    /// 128 bytes shown to the operator.
    pub message_to_show: Vec<u8>,
}

impl Default for CodingAnswer {
    fn default() -> CodingAnswer {
        CodingAnswer {
            operator_id: 0,
            timeout_10ms: 0,
            user_data: vec![0; SZ_DATA_LEN],
            item_expected: false,
            message_to_show: vec![0; 128],
        }
    }
}

/// §8.1 — induction-line belt maintenance. 8 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IlBeltMaintenance {
    pub class_id: u8,
    pub device_id: u16,
    pub belt_id: u8,
    pub command: u8,
    pub time_on_ms: u16,
    pub direction: u8,
}

// ---------------------------------------------------------------------------
// MSC → External Unit (RTXTOOAM)
// ---------------------------------------------------------------------------

/// §4.2.2.1 — **the scanner request**: a cell reached the scanner and
/// the MSC wants a destination. 493 bytes; the message the whole reply
/// path answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannerRequest {
    pub scn_id: u8,
    pub operator_id: u8,
    pub cell: u16,
    pub used_cells: u8,
    pub object_id: u32,
    pub request_type: u8,
    pub request_anomaly: u8,
    pub cell_nif_position: u16,
    /// 240 bytes from the scanner device. The FIRST BYTE classifies the
    /// read (see [`ScannerRequest::read_status`]).
    pub msc_data: Vec<u8>,
    /// 240 bytes from the induction PC — `STRUCT_ISPC_MSC_DATA`, which
    /// carries the parcel's physical dimensions.
    pub user_data: Vec<u8>,
}

impl Default for ScannerRequest {
    fn default() -> ScannerRequest {
        ScannerRequest {
            scn_id: 0,
            operator_id: 0,
            cell: 0,
            used_cells: 0,
            object_id: 0,
            request_type: 0,
            request_anomaly: 0,
            cell_nif_position: 0,
            msc_data: vec![0; SZ_DATA_LEN],
            user_data: vec![0; SZ_DATA_LEN],
        }
    }
}

/// What the first byte of `szMSCData` says about the read (§4.2.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadStatus {
    /// 'T' — the scanner never answered the MSC.
    Timeout,
    /// 'A' — the scanner answered.
    Acknowledged,
    /// SOH (0x01) — a virtual scanner just asking for a destination.
    Virtual,
    /// '!' — no handshake was needed (e.g. a request to reject).
    NoHandshake,
    /// Anything else — scanner-device-specific payload.
    Other(u8),
}

impl ScannerRequest {
    pub fn read_status(&self) -> ReadStatus {
        match self.msc_data.first().copied().unwrap_or(0) {
            b'T' => ReadStatus::Timeout,
            b'A' => ReadStatus::Acknowledged,
            0x01 => ReadStatus::Virtual,
            b'!' => ReadStatus::NoHandshake,
            other => ReadStatus::Other(other),
        }
    }

    /// The induction data, if the block parses as the documented struct.
    pub fn ispc(&self, endian: Endian) -> IspcMscData {
        IspcMscData::decode(&self.user_data, endian)
    }
}

/// §4.2.2.1 — `STRUCT_ISPC_MSC_DATA`, inside the scanner request's
/// `szUserData`. This is where the **parcel physicals** live, which the
/// sort plan's constraints filter on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IspcMscData {
    pub ispc_id: u8,
    pub first_used_cell: u16,
    pub used_cells: u8,
    pub used_gaps: u8,
    pub load_flags: u8,
    pub reject_reason: u16,
    pub object_id: u32,
    pub tracking_id: u32,
    /// Object length on the cell, mm.
    pub length_mm: u16,
    pub width_mm: u16,
    pub height_mm: u16,
    /// Device units.
    pub volume: u32,
    pub weight_g: u16,
    pub head_offset: u16,
    pub destination: u16,
    pub coil_id_master: u16,
    pub antenna_mission_master: u32,
    pub coil_id_backup: u16,
    pub antenna_mission_backup: u32,
    pub statistics: FsgStatisticData,
    /// Plant-custom trailing block.
    pub object_data: Vec<u8>,
}

/// §4.2.2.1 — `STRUCT_FSG_STATISTIC_DATA`, 95 bytes inside the above.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FsgStatisticData {
    /// 2 = ISPC, 3 = conveyor.
    pub subsystem_type: u8,
    pub group_id: u8,
    pub subsystem_id: u8,
    pub zone_id: u8,
    pub scale_id: u8,
    pub volume_id: u8,
    /// Weight "legal for trade".
    pub lftw: u8,
    /// Volume "legal for trade".
    pub lftv: u8,
    pub scale_operative_mode: u8,
    pub volume_operative_mode: u8,
    pub scale_error: u32,
    pub volume_error: u32,
    pub alibi_scale: Vec<u8>,
    pub alibi_volume: Vec<u8>,
}

impl Default for FsgStatisticData {
    fn default() -> FsgStatisticData {
        FsgStatisticData {
            subsystem_type: 0,
            group_id: 0,
            subsystem_id: 0,
            zone_id: 0,
            scale_id: 0,
            volume_id: 0,
            lftw: 0,
            lftv: 0,
            scale_operative_mode: 0,
            volume_operative_mode: 0,
            scale_error: 0,
            volume_error: 0,
            alibi_scale: vec![0; 35],
            alibi_volume: vec![0; 42],
        }
    }
}

impl FsgStatisticData {
    pub const LEN: usize = 95;

    fn read(r: &mut Reader) -> FsgStatisticData {
        FsgStatisticData {
            subsystem_type: r.u8(),
            group_id: r.u8(),
            subsystem_id: r.u8(),
            zone_id: r.u8(),
            scale_id: r.u8(),
            volume_id: r.u8(),
            lftw: r.u8(),
            lftv: r.u8(),
            scale_operative_mode: r.u8(),
            volume_operative_mode: r.u8(),
            scale_error: r.u32(),
            volume_error: r.u32(),
            alibi_scale: r.bytes(35),
            alibi_volume: r.bytes(42),
        }
    }

    fn write(&self, w: &mut Writer) {
        w.u8(self.subsystem_type)
            .u8(self.group_id)
            .u8(self.subsystem_id)
            .u8(self.zone_id)
            .u8(self.scale_id)
            .u8(self.volume_id)
            .u8(self.lftw)
            .u8(self.lftv)
            .u8(self.scale_operative_mode)
            .u8(self.volume_operative_mode)
            .u32(self.scale_error)
            .u32(self.volume_error)
            .block(&self.alibi_scale, 35)
            .block(&self.alibi_volume, 42);
    }
}

impl IspcMscData {
    /// The fixed part, before the plant-custom tail.
    pub const FIXED_LEN: usize = 44 + FsgStatisticData::LEN;

    /// Lenient by design: the block is 240 bytes whose tail is
    /// plant-custom, and a short one still yields the fields it does
    /// carry (zeros beyond). The physicals are what the sort plan needs.
    pub fn decode(data: &[u8], endian: Endian) -> IspcMscData {
        let mut r = Reader::new(data, endian);
        let mut out = IspcMscData {
            ispc_id: r.u8(),
            first_used_cell: r.u16(),
            used_cells: r.u8(),
            used_gaps: r.u8(),
            load_flags: r.u8(),
            reject_reason: r.u16(),
            object_id: r.u32(),
            tracking_id: r.u32(),
            length_mm: r.u16(),
            width_mm: r.u16(),
            height_mm: r.u16(),
            volume: r.u32(),
            weight_g: r.u16(),
            head_offset: r.u16(),
            destination: r.u16(),
            coil_id_master: r.u16(),
            antenna_mission_master: r.u32(),
            coil_id_backup: r.u16(),
            antenna_mission_backup: r.u32(),
            statistics: FsgStatisticData::default(),
            object_data: Vec::new(),
        };
        out.statistics = FsgStatisticData::read(&mut r);
        out.object_data = data.get(r.at.min(data.len())..).unwrap_or(&[]).to_vec();
        out
    }

    /// Render into a `szUserData` block of `len` bytes (240 in the
    /// scanner request).
    pub fn encode(&self, len: usize, endian: Endian) -> Vec<u8> {
        let mut w = Writer::new(endian);
        w.u8(self.ispc_id)
            .u16(self.first_used_cell)
            .u8(self.used_cells)
            .u8(self.used_gaps)
            .u8(self.load_flags)
            .u16(self.reject_reason)
            .u32(self.object_id)
            .u32(self.tracking_id)
            .u16(self.length_mm)
            .u16(self.width_mm)
            .u16(self.height_mm)
            .u32(self.volume)
            .u16(self.weight_g)
            .u16(self.head_offset)
            .u16(self.destination)
            .u16(self.coil_id_master)
            .u32(self.antenna_mission_master)
            .u16(self.coil_id_backup)
            .u32(self.antenna_mission_backup);
        self.statistics.write(&mut w);
        let mut out = w.finish();
        out.extend_from_slice(&self.object_data);
        out.resize(len, 0);
        out
    }
}

/// §4.2.1.1 — **the outcome report**: what actually happened to a
/// parcel. 17 bytes. The message that closes the lifecycle loop.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Outcome {
    pub object_id: u32,
    pub dest: u16,
    pub unload_offset: i8,
    pub cell: u16,
    pub used_cell: u8,
    pub cell_nif_position: u16,
    /// Non-zero = a control zone was reached; zero = the cell is in
    /// front of the destination.
    pub zone: u16,
    /// See [`outcome_return`].
    pub ret: u16,
    /// See [`outcome_anomaly`].
    pub anomaly: u8,
}

/// §10.3 — `wReturn` values of [`Outcome`].
pub mod outcome_return {
    pub const SORTED: u16 = 0;
    pub const REJECTED: u16 = 1;
    pub const REASSIGNEMENT: u16 = 2;
    pub const UNLOAD: u16 = 3;
    pub const PARCELFOUND: u16 = 4;
    pub const PARCELLOST: u16 = 5;
    pub const TO_REJECT: u16 = 6;
    pub const DATA_ERROR: u16 = 7;
    pub const CELL_ACTION_BUILT: u16 = 8;

    /// Names, not numbers — the lifecycle store records what a human
    /// reads.
    pub fn name(v: u16) -> &'static str {
        match v {
            SORTED => "sorted",
            REJECTED => "rejected",
            REASSIGNEMENT => "reassignment",
            UNLOAD => "unload",
            PARCELFOUND => "parcel found",
            PARCELLOST => "parcel lost",
            TO_REJECT => "to reject",
            DATA_ERROR => "data error",
            CELL_ACTION_BUILT => "cell action built",
            _ => "unknown",
        }
    }
}

/// §10.3 — `byAnomaly` values of [`Outcome`].
pub mod outcome_anomaly {
    pub const NOTHING: u8 = 0;
    pub const ENABLE: u8 = 1;
    pub const CELL_FAULT: u8 = 2;
    pub const UNLOADS: u8 = 3;
    pub const CHUTE_FAULT: u8 = 4;
    pub const CHUTE_DISABLED: u8 = 5;
    pub const CHUTE_JAM: u8 = 6;
    pub const CHUTE_OAM_BLOCK: u8 = 7;
    pub const CHUTE_ON_TRACKING: u8 = 8;
    pub const CHUTE_EXTERNAL_BLOCK_SIGNAL: u8 = 9;
    pub const CHUTE_FLAP: u8 = 10;
    /// The chute is full — the signal that must make a destination
    /// temporarily ineligible in the sort plan.
    pub const CHUTE_FULL: u8 = 11;
    /// The MSC never received a destination from us.
    pub const NO_DESTINATION: u8 = 12;
    pub const UNEXPECTED: u8 = 13;
    pub const UNSORTED: u8 = 14;
    pub const NOTREAS: u8 = 15;
    pub const ONGAP: u8 = 16;
    pub const ISPC: u8 = 17;
    pub const CONCURRENT_UNLOADS: u8 = 18;
    pub const UNLOAD_RATE_LIMIT: u8 = 19;
    pub const SPECIAL_NOT_ALLOWED: u8 = 20;
    pub const NOT_DECENTERED: u8 = 21;
    pub const INFLIGHT_RATE_LIMIT: u8 = 22;
    pub const CHUTE_TIMEOUT: u8 = 100;
    /// 101..110 — chute manager refusals 0..9.
    pub const CHUTE_NAK_0: u8 = 101;

    pub fn name(v: u8) -> &'static str {
        match v {
            NOTHING => "none",
            ENABLE => "sorter not ready",
            CELL_FAULT => "cell fault",
            UNLOADS => "unload write failed",
            CHUTE_FAULT => "chute fault",
            CHUTE_DISABLED => "chute disabled",
            CHUTE_JAM => "chute jam",
            CHUTE_OAM_BLOCK => "chute blocked by the external unit",
            CHUTE_ON_TRACKING => "chute under tracking",
            CHUTE_EXTERNAL_BLOCK_SIGNAL => "chute blocked by an external signal",
            CHUTE_FLAP => "chute flap not ready",
            CHUTE_FULL => "chute full",
            NO_DESTINATION => "no destination received",
            UNEXPECTED => "unexpected object",
            UNSORTED => "unsorted object",
            NOTREAS => "never reassigned",
            ONGAP => "loaded between cells",
            ISPC => "rejected by the induction station",
            CONCURRENT_UNLOADS => "too many concurrent unloads",
            UNLOAD_RATE_LIMIT => "unload rate limit",
            SPECIAL_NOT_ALLOWED => "special condition forbids sorting",
            NOT_DECENTERED => "cell not de-centered",
            INFLIGHT_RATE_LIMIT => "in-flight rate limit",
            CHUTE_TIMEOUT => "chute I/O timeout",
            101..=110 => "chute manager refusal",
            _ => "unknown",
        }
    }
}

/// §4.2.1.2 — sorter machine status. 5 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SorterStatus {
    /// 0 unset, 1 maintenance, 2 automatic.
    pub mode: u8,
    /// 0 stopped … 2 running and synchronized.
    pub moto: u8,
    pub transient: u8,
    pub stop_request: u8,
    pub enable_mode: u8,
}

/// §4.2.1.3 — the extended status, with the plant working mode.
/// 6 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SorterStatusEx {
    pub status: SorterStatus,
    pub working_mode: u8,
}

/// §4.2.1.4 — bin status. 7 bytes. `state` is a bit mask; FULL and JAM
/// are the bits the sort plan reacts to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BinStatus {
    pub dest: u16,
    /// 0..8 normal, 9 not-read bin, 10 reject bin.
    pub bin_type: u8,
    pub state: u32,
}

/// §4.2.1.4 — `dwState` bits of [`BinStatus`].
pub mod bin_state {
    pub const RUNNING: u32 = 0x0000_0001;
    pub const EMPTYING: u32 = 0x0000_0002;
    pub const STARTUP: u32 = 0x0000_0004;
    pub const ENERGY_SAVING: u32 = 0x0000_0008;
    pub const MANUAL: u32 = 0x0000_0010;
    pub const MAINTENANCE: u32 = 0x0000_0020;
    pub const AUTORECOVERY: u32 = 0x0000_0040;
    pub const EMERGENCY: u32 = 0x0000_0080;
    pub const EXTERNAL_FAULT: u32 = 0x0000_0100;
    pub const SPEED_OK: u32 = 0x0000_0200;
    pub const JAM: u32 = 0x0000_0400;
    pub const FULL: u32 = 0x0000_0800;
    pub const ALARM: u32 = 0x0000_1000;
    pub const WARNING: u32 = 0x0000_2000;
    pub const HALF_FULL: u32 = 0x0000_4000;
    pub const BOOKED: u32 = 0x0000_8000;
    pub const PREFILLING: u32 = 0x0001_0000;
    pub const COMPLETING: u32 = 0x0002_0000;
    pub const COMPLETED: u32 = 0x0004_0000;
    pub const WAITING_LABEL: u32 = 0x0008_0000;
    pub const DISABLE: u32 = 0x0010_0000;
    pub const BLOCKED_BY_OAM: u32 = 0x0020_0000;
    pub const COMPONENT_FAULT: u32 = 0x0040_0000;
    pub const BLOCKED_BY_EXT: u32 = 0x0080_0000;
    pub const BLOCKED_BY_TRACKING: u32 = 0x0100_0000;
    pub const OPERATOR_PRESENCE: u32 = 0x0200_0000;

    /// Whether a chute in this state can currently receive parcels —
    /// what the sort plan's eligibility check asks.
    pub fn accepts_parcels(state: u32) -> bool {
        const BLOCKING: u32 = JAM
            | FULL
            | ALARM
            | EMERGENCY
            | EXTERNAL_FAULT
            | DISABLE
            | BLOCKED_BY_OAM
            | BLOCKED_BY_EXT
            | BLOCKED_BY_TRACKING
            | MAINTENANCE
            | COMPONENT_FAULT;
        state & BLOCKING == 0
    }

    /// The set bits, as words, for a lifecycle event.
    pub fn names(state: u32) -> Vec<&'static str> {
        BITS.iter()
            .filter(|(bit, _)| state & bit != 0)
            .map(|(_, name)| *name)
            .collect()
    }

    /// Every bit with its name, in bit order — the one table a UI that
    /// lets an operator set a state word builds its checkboxes from.
    pub const BITS: [(u32, &str); 26] = [
        (RUNNING, "running"),
        (EMPTYING, "emptying"),
        (STARTUP, "startup"),
        (ENERGY_SAVING, "energy saving"),
        (MANUAL, "manual"),
        (MAINTENANCE, "maintenance"),
        (AUTORECOVERY, "auto recovery"),
        (EMERGENCY, "emergency"),
        (EXTERNAL_FAULT, "external fault"),
        (SPEED_OK, "speed ok"),
        (JAM, "jam"),
        (FULL, "full"),
        (ALARM, "alarm"),
        (WARNING, "warning"),
        (HALF_FULL, "half full"),
        (BOOKED, "booked"),
        (PREFILLING, "prefilling"),
        (COMPLETING, "completing"),
        (COMPLETED, "completed"),
        (WAITING_LABEL, "waiting label"),
        (DISABLE, "disabled"),
        (BLOCKED_BY_OAM, "blocked by the external unit"),
        (COMPONENT_FAULT, "component fault"),
        (BLOCKED_BY_EXT, "blocked externally"),
        (BLOCKED_BY_TRACKING, "blocked by tracking"),
        (OPERATOR_PRESENCE, "operator present"),
    ];
}

/// §4.2.1.5 — a bin push-button was pressed or released. 10 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BinPushButtons {
    pub button_id: u32,
    pub state: u32,
    pub dest: u16,
}

/// §4.2.1.6 — one per sorter lap, counting 0..255. 1 byte.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SorterLaps {
    pub laps: u8,
}

/// §4.2.1.8 — the MSC is shutting down (main power off). 2 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpsStatus {
    pub ups_status: u16,
}

/// §4.2.2.2 — information from devices along the sorter. 251 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MscDevices {
    pub device_type: u32,
    pub device_id: u8,
    pub cell: u16,
    pub object_id: u32,
    pub msc_data: Vec<u8>,
}

impl Default for MscDevices {
    fn default() -> MscDevices {
        MscDevices {
            device_type: 0,
            device_id: 0,
            cell: 0,
            object_id: 0,
            msc_data: vec![0; SZ_DATA_LEN],
        }
    }
}

/// §4.2.2.3 — a new object passed a control zone. 14 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParcelLoaded {
    pub object_id: u32,
    pub cell: u16,
    pub cell_nif_position: u16,
    pub zone_id: u16,
    pub operator_id: u8,
    pub used_cells: u8,
    pub loaded_type: u8,
    pub loaded_anomaly: u8,
}

/// §4.2.1.10 / §4.2.1.11 — an object made a full lap (recirculated), or
/// passed a control zone. 13 bytes, identical layouts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParcelZoneEvent {
    pub object_id: u32,
    pub cell: u16,
    pub cell_nif_position: u16,
    pub zone_id: u16,
    pub operator_id: u8,
    pub used_cells: u8,
    pub current_anomaly: u8,
}

/// §5.1.1.1 — the operator keyed in a code. 247 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingRequest {
    pub operator_id: u8,
    pub timeout_10ms: u16,
    pub user_data: Vec<u8>,
    pub load_item_anyway: bool,
}

impl Default for CodingRequest {
    fn default() -> CodingRequest {
        CodingRequest {
            operator_id: 0,
            timeout_10ms: 0,
            user_data: vec![0; SZ_DATA_LEN],
            load_item_anyway: false,
        }
    }
}

/// §5.1.1.2 — the manual-coding result. 7 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CodingResult {
    pub operator_id: u8,
    pub item_assigned: bool,
    pub cell: u16,
}

/// §4.2.1.9 — the supervisor asks for a bin lamp test. 4 bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LampTest {
    pub switch_on: bool,
}

/// §6.1 — the generic device-to-device envelope. 506 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenericDataExchange {
    pub sender_msg_id: u16,
    pub sender_who_id: u16,
    pub receiver_msg_id: u16,
    pub receiver_who_id: u16,
    pub data: Vec<u8>,
}

impl Default for GenericDataExchange {
    fn default() -> GenericDataExchange {
        GenericDataExchange {
            sender_msg_id: 0,
            sender_who_id: 0,
            receiver_msg_id: 0,
            receiver_who_id: 0,
            data: vec![0; 498],
        }
    }
}

/// §7.1 — an item was lost on the induction line. 251 bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemLost {
    pub operator_id: u8,
    pub conv_sys_id: u8,
    pub object_id: u32,
    pub lost_position: u8,
    /// 1 auto recovery, 2 tracking lost, 3 alarm, 4 loading (§10.22).
    pub reason: u32,
    pub custom_data: Vec<u8>,
}

impl Default for ItemLost {
    fn default() -> ItemLost {
        ItemLost {
            operator_id: 0,
            conv_sys_id: 0,
            object_id: 0,
            lost_position: 0,
            reason: 0,
            custom_data: vec![0; SZ_DATA_LEN],
        }
    }
}

// ---------------------------------------------------------------------------
// The message union
// ---------------------------------------------------------------------------

/// Any GENIBELT message, typed — plus [`Message::Unknown`] for ids this
/// catalog does not cover (custom plant messages exist; the spec says
/// so). An unknown message is still RECORDED, never dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    // External Unit → MSC
    System(System),
    Commands(Commands),
    CommandsEx(CommandsEx),
    Destination(Destination),
    Lamp(Lamp),
    CodingAnswer(CodingAnswer),
    EuBinState(EuBinState),
    Devices(Devices),
    BinLcdDisplay(BinLcdDisplay),
    OutputSignals(OutputSignals),
    BinStatusAll(Vec<u8>),
    CellReload(CellReload),
    CellReloadEx(CellReloadEx),
    IlBeltMaintenance(IlBeltMaintenance),
    // MSC → External Unit
    Outcome(Outcome),
    SorterStatus(SorterStatus),
    SorterStatusEx(SorterStatusEx),
    BinPushButtons(BinPushButtons),
    BinStatus(BinStatus),
    Scanner(ScannerRequest),
    CodingRequest(CodingRequest),
    CodingResult(CodingResult),
    SorterLaps(SorterLaps),
    UpsStatus(UpsStatus),
    MscDevices(MscDevices),
    ParcelLoaded(ParcelLoaded),
    ParcelRecirculated(ParcelZoneEvent),
    ParcelOnCtrlZone(ParcelZoneEvent),
    InventoryBegin,
    InventoryEnd,
    SorterEmpty,
    // Side families
    LampTest(LampTest),
    GenericDataExchange(GenericDataExchange),
    ItemLost(ItemLost),
    Diagnostic(Vec<u8>),
    /// An id this catalog does not know: kept verbatim so nothing is
    /// lost and the raw frame still round-trips.
    Unknown {
        msg_id: u16,
        data: Vec<u8>,
    },
}

impl Message {
    /// The wire id this message is sent under.
    pub fn msg_id(&self) -> u16 {
        use ids::*;
        match self {
            Message::System(_) => OAMTORTX_SYSTEM,
            Message::Commands(_) => OAMTORTX_COMMANDS,
            Message::CommandsEx(_) => OAMTORTX_COMMANDS_EX,
            Message::Destination(_) => OAMTORTX_DESTINATION,
            Message::Lamp(_) => OAMTORTX_LAMP,
            Message::CodingAnswer(_) => OAMTORTX_CODINGANSWER,
            Message::EuBinState(_) => OAMTORTX_EUBINSTATE,
            Message::Devices(_) => OAMTORTX_DEVICES,
            Message::BinLcdDisplay(_) => OAMTORTX_EUIBINLCDDISPLAY,
            Message::OutputSignals(_) => OAMTORTX_OUTPUTSIGNALS,
            Message::BinStatusAll(_) => OAMTORTX_BINSTATUS_ALL,
            Message::CellReload(_) => OAMTORTX_CELL_RELOAD,
            Message::CellReloadEx(_) => OAMTORTX_CELL_RELOAD_EX,
            Message::IlBeltMaintenance(_) => OAMTORTX_ILBELT_MAINTENANCE,
            Message::Outcome(_) => RTXTOOAM_OUTCOME,
            Message::SorterStatus(_) => RTXTOOAM_SORTERSTATUS,
            Message::SorterStatusEx(_) => RTXTOOAM_SORTERSTATUS_EX,
            Message::BinPushButtons(_) => RTXTOOAM_BINPUSHBUTTONS,
            Message::BinStatus(_) => RTXTOOAM_BINSTATUS,
            Message::Scanner(_) => RTXTOOAM_SCANNER,
            Message::CodingRequest(_) => RTXTOOAM_CODINGREQUEST,
            Message::CodingResult(_) => RTXTOOAM_CODINGRESULT,
            Message::SorterLaps(_) => RTXTOOAM_EVENT_SORTERLAPS,
            Message::UpsStatus(_) => RTXTOOAM_EVENT_UPSSTATUS,
            Message::MscDevices(_) => RTXTOOAM_MSCDEVICES,
            Message::ParcelLoaded(_) => RTXTOOAM_PARCEL_LOADED,
            Message::ParcelRecirculated(_) => RTXTOOAM_PARCEL_RECIRCULATED,
            Message::ParcelOnCtrlZone(_) => RTXTOOAM_PARCEL_ON_CTRLZONE,
            Message::InventoryBegin => RTXTOOAM_INVENTORY_BEGIN,
            Message::InventoryEnd => RTXTOOAM_INVENTORY_END,
            Message::SorterEmpty => RTXTOOAM_SORTER_EMPTY,
            Message::LampTest(_) => CSPCTOEUI_LAMPTEST,
            Message::GenericDataExchange(_) => GENERIC_DATAEXCHANGE,
            Message::ItemLost(_) => CONVSYSTOOAM_ITEMLOST,
            Message::Diagnostic(_) => ISPCTORTX_DIAGNOSTIC,
            Message::Unknown { msg_id, .. } => *msg_id,
        }
    }

    /// A short kind name — what the lifecycle store records as the
    /// event's kind, and what a monitor row shows.
    pub fn kind(&self) -> &'static str {
        match self {
            Message::System(_) => "system",
            Message::Commands(_) | Message::CommandsEx(_) => "command",
            Message::Destination(_) => "destination",
            Message::Lamp(_) => "lamp",
            Message::CodingAnswer(_) => "coding answer",
            Message::EuBinState(_) => "bin state (eu)",
            Message::Devices(_) => "device answer",
            Message::BinLcdDisplay(_) => "bin lcd",
            Message::OutputSignals(_) => "output signal",
            Message::BinStatusAll(_) => "bin status (all)",
            Message::CellReload(_) | Message::CellReloadEx(_) => "cell reload",
            Message::IlBeltMaintenance(_) => "belt maintenance",
            Message::Outcome(_) => "outcome",
            Message::SorterStatus(_) | Message::SorterStatusEx(_) => "sorter status",
            Message::BinPushButtons(_) => "bin push-button",
            Message::BinStatus(_) => "bin status",
            Message::Scanner(_) => "scanner request",
            Message::CodingRequest(_) => "coding request",
            Message::CodingResult(_) => "coding result",
            Message::SorterLaps(_) => "sorter lap",
            Message::UpsStatus(_) => "ups status",
            Message::MscDevices(_) => "sorter device",
            Message::ParcelLoaded(_) => "parcel loaded",
            Message::ParcelRecirculated(_) => "parcel recirculated",
            Message::ParcelOnCtrlZone(_) => "parcel on control zone",
            Message::InventoryBegin => "inventory begin",
            Message::InventoryEnd => "inventory end",
            Message::SorterEmpty => "sorter empty",
            Message::LampTest(_) => "lamp test",
            Message::GenericDataExchange(_) => "generic exchange",
            Message::ItemLost(_) => "item lost",
            Message::Diagnostic(_) => "diagnostic",
            Message::Unknown { .. } => "unknown",
        }
    }

    /// The parcel this message is about, when it is about one.
    ///
    /// This is the **lifecycle key**: how a frame links to a parcel's
    /// timeline in trajetorias's store.
    pub fn object_id(&self) -> Option<u32> {
        match self {
            Message::Destination(m) => Some(m.object_id),
            Message::Devices(m) => Some(m.object_id),
            Message::CellReload(m) => Some(m.object_id),
            Message::CellReloadEx(m) => Some(m.object_id),
            Message::Outcome(m) => Some(m.object_id),
            Message::Scanner(m) => Some(m.object_id),
            Message::MscDevices(m) => Some(m.object_id),
            Message::ParcelLoaded(m) => Some(m.object_id),
            Message::ParcelRecirculated(m) | Message::ParcelOnCtrlZone(m) => Some(m.object_id),
            Message::ItemLost(m) => Some(m.object_id),
            _ => None,
        }
    }

    /// The destination number this message concerns, when it names one.
    pub fn dest(&self) -> Option<u16> {
        match self {
            Message::Destination(m) => Some(m.dest),
            Message::Outcome(m) => Some(m.dest),
            Message::BinStatus(m) => Some(m.dest),
            Message::EuBinState(m) => Some(m.dest),
            Message::BinPushButtons(m) => Some(m.dest),
            Message::BinLcdDisplay(m) => Some(m.dest),
            _ => None,
        }
    }

    /// Decode a data segment's body.
    pub fn decode(msg_id: u16, data: &[u8], endian: Endian) -> Result<Message, MsgError> {
        use ids::*;
        if data.len() > MAX_DATA_SEGMENT {
            return Err(MsgError::TooLong {
                msg_id,
                len: data.len(),
            });
        }
        let mut r = Reader::new(data, endian);
        Ok(match msg_id {
            OAMTORTX_SYSTEM => {
                expect_len(msg_id, data, 2)?;
                Message::System(System { system: r.u16() })
            }
            OAMTORTX_COMMANDS => {
                expect_len(msg_id, data, 10)?;
                Message::Commands(Commands {
                    command: r.u8(),
                    operator_id: r.u8(),
                    param0: r.u8(),
                    param1: r.u8(),
                    param2: r.u16(),
                    param3: r.u32(),
                })
            }
            OAMTORTX_COMMANDS_EX => {
                expect_len(msg_id, data, 15)?;
                Message::CommandsEx(CommandsEx {
                    command: r.u8(),
                    operator_id: r.u8(),
                    param0: r.u8(),
                    param1: r.u8(),
                    param2: r.u16(),
                    param3: r.u32(),
                    class_id: r.u8(),
                    element_id: r.u16(),
                    status_alarm_warning: r.u8(),
                    bit_number: r.u8(),
                })
            }
            OAMTORTX_DESTINATION => {
                expect_len(msg_id, data, 46)?;
                Message::Destination(Destination {
                    object_id: r.u32(),
                    dest: r.u16(),
                    cell: r.u16(),
                    used_cells: r.u8(),
                    flag: r.u8(),
                    scn_id: r.u8(),
                    operator_id: r.u8(),
                    ramp_id: r.u8(),
                    dest_unload_offset: r.i8(),
                    oam_reject_anomaly: r.u8(),
                    num_alternative_dest: r.u8(),
                    alternative_dest: r.array_u16(10),
                    alternative_dest_unload_offset: r.array_i8(10),
                })
            }
            OAMTORTX_LAMP => {
                expect_len(msg_id, data, 14)?;
                Message::Lamp(Lamp {
                    device_id: r.u16(),
                    element: r.u16(),
                    command: r.u8(),
                    inhibit_unload: r.bool32(),
                    lamp_type: r.u8(),
                    time_on_ms: r.u16(),
                    time_off_ms: r.u16(),
                })
            }
            OAMTORTX_CODINGANSWER => {
                expect_len(msg_id, data, 375)?;
                Message::CodingAnswer(CodingAnswer {
                    operator_id: r.u8(),
                    timeout_10ms: r.u16(),
                    user_data: r.bytes(SZ_DATA_LEN),
                    item_expected: r.bool32(),
                    message_to_show: r.bytes(128),
                })
            }
            OAMTORTX_EUBINSTATE => {
                expect_len(msg_id, data, 6)?;
                Message::EuBinState(EuBinState {
                    dest: r.u16(),
                    state: r.u32(),
                })
            }
            OAMTORTX_DEVICES => {
                expect_len(msg_id, data, 12)?;
                Message::Devices(Devices {
                    device_type: r.u32(),
                    device_id: r.u8(),
                    cell: r.u16(),
                    flag: r.u8(),
                    object_id: r.u32(),
                })
            }
            OAMTORTX_EUIBINLCDDISPLAY => {
                expect_len(msg_id, data, 82)?;
                let dest = r.u16();
                let text = utf16z(&r.bytes(80), endian);
                Message::BinLcdDisplay(BinLcdDisplay { dest, text })
            }
            OAMTORTX_OUTPUTSIGNALS => {
                expect_len(msg_id, data, 34)?;
                Message::OutputSignals(OutputSignals {
                    class_id: r.u8(),
                    device_id: r.u16(),
                    output_name: asciiz(&r.bytes(26)),
                    output_state: r.u8(),
                    time_on_ms: r.u16(),
                    time_off_ms: r.u16(),
                })
            }
            // Plant-specific payload; kept verbatim.
            OAMTORTX_BINSTATUS_ALL => Message::BinStatusAll(data.to_vec()),
            OAMTORTX_CELL_RELOAD => {
                if data.len() < 8 {
                    return Err(MsgError::WrongLength {
                        msg_id,
                        expected: 8,
                        actual: data.len(),
                    });
                }
                Message::CellReload(CellReload {
                    cell: r.u16(),
                    used_cells: r.u8(),
                    ispc_id: r.u8(),
                    object_id: r.u32(),
                    custom_data: data[8..].to_vec(),
                })
            }
            OAMTORTX_CELL_RELOAD_EX => {
                expect_len(msg_id, data, 249)?;
                Message::CellReloadEx(CellReloadEx {
                    cell: r.u16(),
                    used_cells: r.u8(),
                    ispc_id: r.u8(),
                    rescan_required: r.u8(),
                    object_id: r.u32(),
                    user_data: r.bytes(SZ_DATA_LEN),
                })
            }
            OAMTORTX_ILBELT_MAINTENANCE => {
                expect_len(msg_id, data, 8)?;
                Message::IlBeltMaintenance(IlBeltMaintenance {
                    class_id: r.u8(),
                    device_id: r.u16(),
                    belt_id: r.u8(),
                    command: r.u8(),
                    time_on_ms: r.u16(),
                    direction: r.u8(),
                })
            }
            RTXTOOAM_OUTCOME => {
                expect_len(msg_id, data, 17)?;
                Message::Outcome(Outcome {
                    object_id: r.u32(),
                    dest: r.u16(),
                    unload_offset: r.i8(),
                    cell: r.u16(),
                    used_cell: r.u8(),
                    cell_nif_position: r.u16(),
                    zone: r.u16(),
                    ret: r.u16(),
                    anomaly: r.u8(),
                })
            }
            RTXTOOAM_SORTERSTATUS => {
                expect_len(msg_id, data, 5)?;
                Message::SorterStatus(SorterStatus {
                    mode: r.u8(),
                    moto: r.u8(),
                    transient: r.u8(),
                    stop_request: r.u8(),
                    enable_mode: r.u8(),
                })
            }
            RTXTOOAM_SORTERSTATUS_EX => {
                expect_len(msg_id, data, 6)?;
                Message::SorterStatusEx(SorterStatusEx {
                    status: SorterStatus {
                        mode: r.u8(),
                        moto: r.u8(),
                        transient: r.u8(),
                        stop_request: r.u8(),
                        enable_mode: r.u8(),
                    },
                    working_mode: r.u8(),
                })
            }
            RTXTOOAM_BINPUSHBUTTONS => {
                expect_len(msg_id, data, 10)?;
                Message::BinPushButtons(BinPushButtons {
                    button_id: r.u32(),
                    state: r.u32(),
                    dest: r.u16(),
                })
            }
            RTXTOOAM_BINSTATUS => {
                expect_len(msg_id, data, 7)?;
                Message::BinStatus(BinStatus {
                    dest: r.u16(),
                    bin_type: r.u8(),
                    state: r.u32(),
                })
            }
            RTXTOOAM_SCANNER => {
                expect_len(msg_id, data, 493)?;
                Message::Scanner(ScannerRequest {
                    scn_id: r.u8(),
                    operator_id: r.u8(),
                    cell: r.u16(),
                    used_cells: r.u8(),
                    object_id: r.u32(),
                    request_type: r.u8(),
                    request_anomaly: r.u8(),
                    cell_nif_position: r.u16(),
                    msc_data: r.bytes(SZ_DATA_LEN),
                    user_data: r.bytes(SZ_DATA_LEN),
                })
            }
            RTXTOOAM_CODINGREQUEST => {
                expect_len(msg_id, data, 247)?;
                Message::CodingRequest(CodingRequest {
                    operator_id: r.u8(),
                    timeout_10ms: r.u16(),
                    user_data: r.bytes(SZ_DATA_LEN),
                    load_item_anyway: r.bool32(),
                })
            }
            RTXTOOAM_CODINGRESULT => {
                expect_len(msg_id, data, 7)?;
                Message::CodingResult(CodingResult {
                    operator_id: r.u8(),
                    item_assigned: r.bool32(),
                    cell: r.u16(),
                })
            }
            RTXTOOAM_EVENT_SORTERLAPS => {
                expect_len(msg_id, data, 1)?;
                Message::SorterLaps(SorterLaps { laps: r.u8() })
            }
            RTXTOOAM_EVENT_UPSSTATUS => {
                expect_len(msg_id, data, 2)?;
                Message::UpsStatus(UpsStatus {
                    ups_status: r.u16(),
                })
            }
            RTXTOOAM_MSCDEVICES => {
                expect_len(msg_id, data, 251)?;
                Message::MscDevices(MscDevices {
                    device_type: r.u32(),
                    device_id: r.u8(),
                    cell: r.u16(),
                    object_id: r.u32(),
                    msc_data: r.bytes(SZ_DATA_LEN),
                })
            }
            RTXTOOAM_PARCEL_LOADED => {
                expect_len(msg_id, data, 14)?;
                Message::ParcelLoaded(ParcelLoaded {
                    object_id: r.u32(),
                    cell: r.u16(),
                    cell_nif_position: r.u16(),
                    zone_id: r.u16(),
                    operator_id: r.u8(),
                    used_cells: r.u8(),
                    loaded_type: r.u8(),
                    loaded_anomaly: r.u8(),
                })
            }
            RTXTOOAM_PARCEL_RECIRCULATED | RTXTOOAM_PARCEL_ON_CTRLZONE => {
                expect_len(msg_id, data, 13)?;
                let event = ParcelZoneEvent {
                    object_id: r.u32(),
                    cell: r.u16(),
                    cell_nif_position: r.u16(),
                    zone_id: r.u16(),
                    operator_id: r.u8(),
                    used_cells: r.u8(),
                    current_anomaly: r.u8(),
                };
                if msg_id == RTXTOOAM_PARCEL_RECIRCULATED {
                    Message::ParcelRecirculated(event)
                } else {
                    Message::ParcelOnCtrlZone(event)
                }
            }
            RTXTOOAM_INVENTORY_BEGIN => {
                expect_len(msg_id, data, 0)?;
                Message::InventoryBegin
            }
            RTXTOOAM_INVENTORY_END => {
                expect_len(msg_id, data, 0)?;
                Message::InventoryEnd
            }
            RTXTOOAM_SORTER_EMPTY => {
                expect_len(msg_id, data, 0)?;
                Message::SorterEmpty
            }
            CSPCTOEUI_LAMPTEST => {
                expect_len(msg_id, data, 4)?;
                Message::LampTest(LampTest {
                    switch_on: r.bool32(),
                })
            }
            GENERIC_DATAEXCHANGE => {
                expect_len(msg_id, data, 506)?;
                Message::GenericDataExchange(GenericDataExchange {
                    sender_msg_id: r.u16(),
                    sender_who_id: r.u16(),
                    receiver_msg_id: r.u16(),
                    receiver_who_id: r.u16(),
                    data: r.bytes(498),
                })
            }
            CONVSYSTOOAM_ITEMLOST => {
                expect_len(msg_id, data, 251)?;
                Message::ItemLost(ItemLost {
                    operator_id: r.u8(),
                    conv_sys_id: r.u8(),
                    object_id: r.u32(),
                    lost_position: r.u8(),
                    reason: r.u32(),
                    custom_data: r.bytes(SZ_DATA_LEN),
                })
            }
            ISPCTORTX_DIAGNOSTIC => Message::Diagnostic(data.to_vec()),
            other => Message::Unknown {
                msg_id: other,
                data: data.to_vec(),
            },
        })
    }

    /// Render this message's body.
    pub fn encode(&self, endian: Endian) -> Result<Vec<u8>, MsgError> {
        let mut w = Writer::new(endian);
        let out = match self {
            Message::System(m) => w.u16(m.system).finish(),
            Message::Commands(m) => w
                .u8(m.command)
                .u8(m.operator_id)
                .u8(m.param0)
                .u8(m.param1)
                .u16(m.param2)
                .u32(m.param3)
                .finish(),
            Message::CommandsEx(m) => w
                .u8(m.command)
                .u8(m.operator_id)
                .u8(m.param0)
                .u8(m.param1)
                .u16(m.param2)
                .u32(m.param3)
                .u8(m.class_id)
                .u16(m.element_id)
                .u8(m.status_alarm_warning)
                .u8(m.bit_number)
                .finish(),
            Message::Destination(m) => w
                .u32(m.object_id)
                .u16(m.dest)
                .u16(m.cell)
                .u8(m.used_cells)
                .u8(m.flag)
                .u8(m.scn_id)
                .u8(m.operator_id)
                .u8(m.ramp_id)
                .i8(m.dest_unload_offset)
                .u8(m.oam_reject_anomaly)
                .u8(m.num_alternative_dest)
                .array_u16(&m.alternative_dest, 10)
                .array_i8(&m.alternative_dest_unload_offset, 10)
                .finish(),
            Message::Lamp(m) => w
                .u16(m.device_id)
                .u16(m.element)
                .u8(m.command)
                .bool32(m.inhibit_unload)
                .u8(m.lamp_type)
                .u16(m.time_on_ms)
                .u16(m.time_off_ms)
                .finish(),
            Message::CodingAnswer(m) => w
                .u8(m.operator_id)
                .u16(m.timeout_10ms)
                .block(&m.user_data, SZ_DATA_LEN)
                .bool32(m.item_expected)
                .block(&m.message_to_show, 128)
                .finish(),
            Message::EuBinState(m) => w.u16(m.dest).u32(m.state).finish(),
            Message::Devices(m) => w
                .u32(m.device_type)
                .u8(m.device_id)
                .u16(m.cell)
                .u8(m.flag)
                .u32(m.object_id)
                .finish(),
            Message::BinLcdDisplay(m) => {
                let text = utf16_block(&m.text, 80, endian);
                w.u16(m.dest).block(&text, 80).finish()
            }
            Message::OutputSignals(m) => w
                .u8(m.class_id)
                .u16(m.device_id)
                .block(m.output_name.as_bytes(), 26)
                .u8(m.output_state)
                .u16(m.time_on_ms)
                .u16(m.time_off_ms)
                .finish(),
            Message::BinStatusAll(data) => data.clone(),
            Message::CellReload(m) => {
                let mut out = w
                    .u16(m.cell)
                    .u8(m.used_cells)
                    .u8(m.ispc_id)
                    .u32(m.object_id)
                    .finish();
                out.extend_from_slice(&m.custom_data);
                out
            }
            Message::CellReloadEx(m) => w
                .u16(m.cell)
                .u8(m.used_cells)
                .u8(m.ispc_id)
                .u8(m.rescan_required)
                .u32(m.object_id)
                .block(&m.user_data, SZ_DATA_LEN)
                .finish(),
            Message::IlBeltMaintenance(m) => w
                .u8(m.class_id)
                .u16(m.device_id)
                .u8(m.belt_id)
                .u8(m.command)
                .u16(m.time_on_ms)
                .u8(m.direction)
                .finish(),
            Message::Outcome(m) => w
                .u32(m.object_id)
                .u16(m.dest)
                .i8(m.unload_offset)
                .u16(m.cell)
                .u8(m.used_cell)
                .u16(m.cell_nif_position)
                .u16(m.zone)
                .u16(m.ret)
                .u8(m.anomaly)
                .finish(),
            Message::SorterStatus(m) => w
                .u8(m.mode)
                .u8(m.moto)
                .u8(m.transient)
                .u8(m.stop_request)
                .u8(m.enable_mode)
                .finish(),
            Message::SorterStatusEx(m) => w
                .u8(m.status.mode)
                .u8(m.status.moto)
                .u8(m.status.transient)
                .u8(m.status.stop_request)
                .u8(m.status.enable_mode)
                .u8(m.working_mode)
                .finish(),
            Message::BinPushButtons(m) => w.u32(m.button_id).u32(m.state).u16(m.dest).finish(),
            Message::BinStatus(m) => w.u16(m.dest).u8(m.bin_type).u32(m.state).finish(),
            Message::Scanner(m) => w
                .u8(m.scn_id)
                .u8(m.operator_id)
                .u16(m.cell)
                .u8(m.used_cells)
                .u32(m.object_id)
                .u8(m.request_type)
                .u8(m.request_anomaly)
                .u16(m.cell_nif_position)
                .block(&m.msc_data, SZ_DATA_LEN)
                .block(&m.user_data, SZ_DATA_LEN)
                .finish(),
            Message::CodingRequest(m) => w
                .u8(m.operator_id)
                .u16(m.timeout_10ms)
                .block(&m.user_data, SZ_DATA_LEN)
                .bool32(m.load_item_anyway)
                .finish(),
            Message::CodingResult(m) => w
                .u8(m.operator_id)
                .bool32(m.item_assigned)
                .u16(m.cell)
                .finish(),
            Message::SorterLaps(m) => w.u8(m.laps).finish(),
            Message::UpsStatus(m) => w.u16(m.ups_status).finish(),
            Message::MscDevices(m) => w
                .u32(m.device_type)
                .u8(m.device_id)
                .u16(m.cell)
                .u32(m.object_id)
                .block(&m.msc_data, SZ_DATA_LEN)
                .finish(),
            Message::ParcelLoaded(m) => w
                .u32(m.object_id)
                .u16(m.cell)
                .u16(m.cell_nif_position)
                .u16(m.zone_id)
                .u8(m.operator_id)
                .u8(m.used_cells)
                .u8(m.loaded_type)
                .u8(m.loaded_anomaly)
                .finish(),
            Message::ParcelRecirculated(m) | Message::ParcelOnCtrlZone(m) => w
                .u32(m.object_id)
                .u16(m.cell)
                .u16(m.cell_nif_position)
                .u16(m.zone_id)
                .u8(m.operator_id)
                .u8(m.used_cells)
                .u8(m.current_anomaly)
                .finish(),
            Message::InventoryBegin | Message::InventoryEnd | Message::SorterEmpty => Vec::new(),
            Message::LampTest(m) => w.bool32(m.switch_on).finish(),
            Message::GenericDataExchange(m) => w
                .u16(m.sender_msg_id)
                .u16(m.sender_who_id)
                .u16(m.receiver_msg_id)
                .u16(m.receiver_who_id)
                .block(&m.data, 498)
                .finish(),
            Message::ItemLost(m) => w
                .u8(m.operator_id)
                .u8(m.conv_sys_id)
                .u32(m.object_id)
                .u8(m.lost_position)
                .u32(m.reason)
                .block(&m.custom_data, SZ_DATA_LEN)
                .finish(),
            Message::Diagnostic(data) => data.clone(),
            Message::Unknown { data, .. } => data.clone(),
        };
        if out.len() > MAX_DATA_SEGMENT {
            return Err(MsgError::TooLong {
                msg_id: self.msg_id(),
                len: out.len(),
            });
        }
        Ok(out)
    }
}
