//! The cyclic real-time frame of PROFINET IO: what an IO controller and an IO device exchange every
//! cycle once a connection is up.
//!
//! ```text
//! [802.1Q tag, priority 6] EtherType 0x8892 | FrameID u16 | C_SDU (40 to 1440 bytes) |
//!   CycleCounter u16 | DataStatus u8 | TransferStatus u8
//! ```
//!
//! The C_SDU holds each submodule's data followed by its provider status byte (IOPS) and the
//! consumer status bytes (IOCS) of what flows the other way, at the offsets the connection set up;
//! this module carries it as bytes and reads one status byte, never the layout, which is the
//! connection's. The frame ids are read by range, as Wireshark's dissector reads them: RT class 3
//! from 0x0100 to 0x0FFF, RT class 1 from 0x8000 to 0xBFFF, its legacy range from 0xC000 to 0xFBFF,
//! the two alarm ids, DCP's four; everything else is not a cyclic frame.

use serde::{Deserialize, Serialize};

use super::ether::{self, ETHERTYPE_PROFINET, EtherError, VlanTag};
use super::mac::Mac;

/// The shortest and longest C_SDU.
pub const MIN_DATA: usize = 40;
pub const MAX_DATA: usize = 1440;

/// The tag a cyclic frame rides with: priority 6, VLAN 0 (priority only).
pub const CYCLIC_TAG: VlanTag = VlanTag { priority: 6, drop_eligible: false, id: 0 };

/// What a frame id names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FrameClass {
    /// 0x0100 to 0x0FFF: scheduled (isochronous) cyclic data.
    Rtc3,
    /// 0x8000 to 0xBFFF: cyclic data.
    Rtc1,
    /// 0xC000 to 0xFBFF: cyclic data in the older range.
    Rtc1Legacy,
    /// 0xFC01: an alarm of high priority.
    AlarmHigh,
    /// 0xFE01: an alarm of low priority.
    AlarmLow,
    /// 0xFEFC to 0xFEFF.
    Dcp,
    /// Anything else: reserved, time synchronisation, or not PROFINET's to give.
    Other,
}

impl FrameClass {
    pub fn of(frame_id: u16) -> FrameClass {
        match frame_id {
            0x0100..=0x0FFF => FrameClass::Rtc3,
            0x8000..=0xBFFF => FrameClass::Rtc1,
            0xC000..=0xFBFF => FrameClass::Rtc1Legacy,
            0xFC01 => FrameClass::AlarmHigh,
            0xFE01 => FrameClass::AlarmLow,
            0xFEFC..=0xFEFF => FrameClass::Dcp,
            _ => FrameClass::Other,
        }
    }

    /// A frame of cyclic IO data.
    pub fn is_cyclic(self) -> bool {
        matches!(self, FrameClass::Rtc3 | FrameClass::Rtc1 | FrameClass::Rtc1Legacy)
    }
}

/// The APDU's data status byte, as it travels, with its bits named.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataStatus(pub u8);

impl DataStatus {
    /// A provider in RUN with valid data, primary, its station without a problem: 0x35.
    pub const GOOD: DataStatus = DataStatus(0x35);

    fn bit(self, n: u8) -> bool {
        self.0 & (1 << n) != 0
    }

    /// Bit 0: the primary provider (not the backup of a redundant pair).
    pub fn primary(self) -> bool {
        self.bit(0)
    }

    /// Bit 1: a redundant provider has no primary.
    pub fn redundancy(self) -> bool {
        self.bit(1)
    }

    /// Bit 2: the data is valid.
    pub fn valid(self) -> bool {
        self.bit(2)
    }

    /// Bit 4: the provider is in RUN (not STOP).
    pub fn running(self) -> bool {
        self.bit(4)
    }

    /// Bit 5: the station has no problem (the station problem indicator says normal).
    pub fn station_ok(self) -> bool {
        self.bit(5)
    }

    /// Bit 7: the consumer is to ignore the frame's data. (Bits 3 and 6 are reserved.)
    pub fn ignore(self) -> bool {
        self.bit(7)
    }
}

/// A provider or consumer status byte of the C_SDU (IOPS, IOCS): good or bad, and who decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IoxStatus(pub u8);

impl IoxStatus {
    /// Good, decided by the subslot: 0x80.
    pub const GOOD: IoxStatus = IoxStatus(0x80);
    /// Bad, decided by the subslot: 0x00.
    pub const BAD: IoxStatus = IoxStatus(0x00);

    /// Bit 7: the data is good.
    pub fn good(self) -> bool {
        self.0 & 0x80 != 0
    }

    /// Bits 5 and 6: who detected the state: 0 the subslot, 1 the slot, 2 the device, 3 the
    /// controller.
    pub fn instance(self) -> u8 {
        (self.0 >> 5) & 0x03
    }

    /// Bit 0: another status byte follows.
    pub fn extension(self) -> bool {
        self.0 & 1 != 0
    }
}

/// One cyclic frame: its id, its C_SDU as bytes, and its APDU status.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CyclicFrame {
    pub frame_id: u16,
    pub data: Vec<u8>,
    pub cycle_counter: u16,
    pub data_status: DataStatus,
    pub transfer_status: u8,
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum RtError {
    #[error("frame id {0:#06x} is not a cyclic frame's")]
    NotCyclic(u16),
    #[error("a cyclic frame's data is {MIN_DATA} to {MAX_DATA} bytes; this one is {0}")]
    DataLength(usize),
    #[error("{0}")]
    Ether(#[from] EtherError),
}

impl CyclicFrame {
    /// Read a cyclic frame from the PROFINET payload (the bytes after the EtherType, without the
    /// frame check sequence). The APDU status is the last four bytes: a cyclic frame is never
    /// shorter than an Ethernet frame, so it carries no pad.
    pub fn decode(payload: &[u8]) -> Result<CyclicFrame, RtError> {
        if payload.len() < 2 {
            return Err(RtError::DataLength(0));
        }
        let frame_id = u16::from_be_bytes([payload[0], payload[1]]);
        if !FrameClass::of(frame_id).is_cyclic() {
            return Err(RtError::NotCyclic(frame_id));
        }
        let data_len = payload.len().saturating_sub(6);
        if payload.len() < 6 || !(MIN_DATA..=MAX_DATA).contains(&data_len) {
            return Err(RtError::DataLength(data_len));
        }
        let status = &payload[payload.len() - 4..];
        Ok(CyclicFrame {
            frame_id,
            data: payload[2..payload.len() - 4].to_vec(),
            cycle_counter: u16::from_be_bytes([status[0], status[1]]),
            data_status: DataStatus(status[2]),
            transfer_status: status[3],
        })
    }

    /// The PROFINET payload: frame id, data, APDU status. A frame id that is not a cyclic frame's,
    /// or data outside 40 to 1440 bytes, is refused (the connection pads its data to 40 itself).
    pub fn encode(&self) -> Result<Vec<u8>, RtError> {
        if !FrameClass::of(self.frame_id).is_cyclic() {
            return Err(RtError::NotCyclic(self.frame_id));
        }
        if !(MIN_DATA..=MAX_DATA).contains(&self.data.len()) {
            return Err(RtError::DataLength(self.data.len()));
        }
        let mut out = Vec::with_capacity(self.data.len() + 6);
        out.extend_from_slice(&self.frame_id.to_be_bytes());
        out.extend_from_slice(&self.data);
        out.extend_from_slice(&self.cycle_counter.to_be_bytes());
        out.push(self.data_status.0);
        out.push(self.transfer_status);
        Ok(out)
    }

    /// The whole Ethernet frame, with the cyclic tag (priority 6, VLAN 0) or the tag given.
    pub fn frame(&self, dst: Mac, src: Mac, tag: Option<VlanTag>) -> Result<Vec<u8>, RtError> {
        Ok(ether::build_tagged(dst, src, tag, ETHERTYPE_PROFINET, &self.encode()?)?)
    }
}
