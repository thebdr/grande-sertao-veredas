//! The wire: the one seam every frame aranha hears or sends passes through (`docs/aranha-plan.md`,
//! "Scope"). Three kinds sit behind it: the raw link on a named interface (`raw`, Linux), a
//! capture file read back in order (`pcap::Replay`), and the memory hub the pure tests share
//! (`hub`). Everything above the seam (the listener, fusion, asking) knows none of them.
//!
//! A wire hands back the frames that arrived on its interface, never one this host sent: a
//! listener keeps its own sends itself, so what it heard and what it said stay apart.

pub mod hub;
pub mod pcap;
#[cfg(target_os = "linux")]
pub mod raw;

use std::time::{Duration, SystemTime};

use crate::profinet::mac::Mac;

/// The longest frame a wire hands back or takes to send, its tags included.
pub const MAX_FRAME: usize = 65_536;

/// One frame as a wire hands it: its bytes as they were on the wire, the interface that heard it,
/// when (nanoseconds since the Unix epoch), and its length on the wire, more than its bytes when a
/// capture kept only the first of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub at_ns: u64,
    pub iface: String,
    pub bytes: Vec<u8>,
    pub length: usize,
}

impl Frame {
    /// A whole frame: its length on the wire is its bytes'.
    pub fn new(at_ns: u64, iface: impl Into<String>, bytes: Vec<u8>) -> Frame {
        let length = bytes.len();
        Frame { at_ns, iface: iface.into(), bytes, length }
    }

    /// Whether the capture that kept it kept fewer bytes than the wire carried.
    pub fn is_cut(&self) -> bool {
        self.length > self.bytes.len()
    }
}

/// A clock reading as a frame's stamp: nanoseconds since the Unix epoch, 0 before it, and the
/// largest stamp past what 64 bits count (the year 2554).
pub fn stamp(at: SystemTime) -> u64 {
    at.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("{0} has no frame left: its capture is read to the end")]
    Ended(String),
    #[error("a frame of {0} bytes is shorter than an Ethernet header")]
    TooShort(usize),
    #[error("a frame of {0} bytes is past the {MAX_FRAME} a wire carries")]
    TooLong(usize),
    #[cfg(target_os = "linux")]
    #[error(transparent)]
    Raw(#[from] raw::RawError),
}

/// Where frames are sent and heard. Every call is bounded: `recv` waits at most its timeout (a
/// capture never waits), `send` never blocks.
pub trait Wire: Send {
    /// The interface's name, or the capture's.
    fn name(&self) -> &str;
    /// The interface's own MAC; a capture has none.
    fn mac(&self) -> Option<Mac>;
    /// Send one whole frame, its Ethernet header included.
    fn send(&mut self, frame: &[u8]) -> Result<(), WireError>;
    /// The next frame that arrived, or `None` when none did within `timeout`.
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError>;
}

/// A frame's length checked as every wire checks it before sending.
fn sendable(frame: &[u8]) -> Result<(), WireError> {
    if frame.len() < 14 {
        return Err(WireError::TooShort(frame.len()));
    }
    if frame.len() > MAX_FRAME {
        return Err(WireError::TooLong(frame.len()));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
impl Wire for raw::RawLink {
    fn name(&self) -> &str {
        self.iface()
    }

    fn mac(&self) -> Option<Mac> {
        Some(raw::RawLink::mac(self))
    }

    fn send(&mut self, frame: &[u8]) -> Result<(), WireError> {
        sendable(frame)?;
        Ok(raw::RawLink::send(self, frame)?)
    }

    /// A frame this host sent on the interface (a link opened to hear them) is passed over within
    /// the same wait.
    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        let deadline = std::time::Instant::now().checked_add(timeout);
        loop {
            let left = deadline.map_or(Duration::MAX, |d| d.saturating_duration_since(std::time::Instant::now()));
            match raw::RawLink::recv(self, left)? {
                None => return Ok(None),
                // Passed over: the wait goes on to the same deadline (none left, a last look).
                Some(r) if r.outgoing => {}
                Some(r) => {
                    let length = r.bytes.len();
                    return Ok(Some(Frame { at_ns: stamp(r.at), iface: self.iface().to_string(), bytes: r.bytes, length }));
                }
            }
        }
    }
}
