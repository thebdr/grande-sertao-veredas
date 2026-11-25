//! Asking (`docs/aranha-plan.md`, "Scope", asking; "Rules": it listens before it asks, every active
//! step bounded and paced). A DCP Identify (all, by station name, by alias) is sent through a
//! listener already hearing its interface, from that interface's MAC, paced by that interface's
//! worker like any frame it sends. Its answers are the Identify answers that listener then keeps
//! from that interface, addressed to that MAC under the request's transaction id; the listener has
//! fused them already, each value with its witness, so asking adds no other way into the devices.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::listen::{ListenError, Listener, Outcome};
use crate::profinet::dcp::{self, DcpError, DeviceIdentity, Filter, Service};
use crate::profinet::ether::{self, ETHERTYPE_PROFINET};
use crate::profinet::mac::Mac;

/// DCP's largest response delay factor (6400, 64 s).
pub const MAX_RESPONSE_DELAY: u16 = 6400;
/// The longest an answer is waited for past the devices' spread.
pub const MAX_WAIT: Duration = Duration::from_secs(60);

/// What to ask, and how long to wait.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identify {
    pub filter: Filter,
    /// The factor the devices spread their answers over, in 10 ms steps (DCP's 1 to 6400; 1
    /// answers at once).
    pub response_delay: u16,
    /// How long past the spread an answer is still waited for.
    pub wait: Duration,
}

impl Default for Identify {
    /// Every device, answering at once (Veredas 1's Identify-All), waited for a second.
    fn default() -> Self {
        Identify { filter: Filter::All, response_delay: 1, wait: Duration::from_secs(1) }
    }
}

/// One device's answer: who, in which kept frame, when, and what it said of itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub mac: Mac,
    pub frame: u64,
    pub at_ns: u64,
    pub identity: DeviceIdentity,
}

/// An Identify asked, and what came of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asked {
    pub iface: String,
    pub xid: u32,
    /// The request as the listener kept it: its index and its stamp.
    pub request: u64,
    pub sent_ns: u64,
    /// What it waited after the request went: the devices' spread and its wait.
    pub window: Duration,
    /// The answers to it, in the order heard.
    pub answers: Vec<Answer>,
    /// Identify answers heard on the interface meanwhile that are not to it: another transaction
    /// id, or another station's.
    pub others: u64,
    /// Frames after the request let go past the listener's bound before they were read.
    pub lost: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum AskError {
    #[error("{0}")]
    Listen(#[from] ListenError),
    #[error("{0:?} has no MAC of its own (a capture replayed): nothing is asked from it")]
    NoMac(String),
    #[error("{iface:?} is heard no more ({reason}): nothing is asked on it")]
    Ended { iface: String, reason: String },
    #[error("a response delay of {0} is outside DCP's 1 to 6400")]
    Delay(u16),
    #[error("a wait of {0:?} is past the minute an answer is waited for")]
    Wait(Duration),
    #[error("the Identify cannot be written: {0}")]
    Request(DcpError),
    #[error("{iface:?}'s wire refused the Identify: {reason}")]
    Refused { iface: String, reason: String },
    #[error("{iface:?} did not send the Identify within its turn, {waited:?}: it is withdrawn and never sent")]
    NotSent { iface: String, waited: Duration },
    #[error("{iface:?}'s wire was still taking the Identify past its turn, {waited:?}: it may yet go, and no answer to it is this ask's")]
    Stalled { iface: String, waited: Duration },
}

/// A transaction id this process has not used: a count from a start drawn from the clock and the
/// process, so two scanners on one interface do not share one.
fn next_xid() -> u32 {
    static BASE: OnceLock<u32> = OnceLock::new();
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let base = *BASE.get_or_init(|| {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() ^ d.as_secs() as u32);
        nanos ^ std::process::id().rotate_left(16)
    });
    base.wrapping_add(COUNT.fetch_add(1, Ordering::Relaxed))
}

/// Ask `ask` on `iface` through `listener`, and wait for the answers: its turn in the interface's
/// queue, the devices' spread (the response delay's factor in 10 ms steps), then `ask.wait`.
pub fn identify(listener: &Listener, iface: &str, ask: &Identify) -> Result<Asked, AskError> {
    if !(1..=MAX_RESPONSE_DELAY).contains(&ask.response_delay) {
        return Err(AskError::Delay(ask.response_delay));
    }
    if ask.wait > MAX_WAIT {
        return Err(AskError::Wait(ask.wait));
    }
    let account = listener.interface(iface).ok_or_else(|| ListenError::NotListening(iface.to_string()))?;
    if let Some(reason) = account.ended {
        return Err(AskError::Ended { iface: iface.to_string(), reason });
    }
    let mac = account.mac.ok_or_else(|| AskError::NoMac(iface.to_string()))?;
    let xid = next_xid();
    let request = dcp::identify_request(mac, xid, ask.response_delay, &ask.filter).map_err(AskError::Request)?;
    let tracked = listener.send_tracked(iface, request)?;

    // Its turn: the frames queued ahead of it (counted as it was queued) at the interface's pace,
    // and a second more; past it, taken back if it still waits.
    let gap = Duration::from_secs(1) / listener.options().rate;
    let waited = gap.saturating_mul(u32::try_from(tracked.ahead + 1).unwrap_or(u32::MAX)).saturating_add(Duration::from_secs(1));
    let (index, sent_ns) = match tracked.told.recv_timeout(waited) {
        Ok(Outcome::Sent { index, at_ns }) => (index, at_ns),
        Ok(Outcome::Refused(reason)) => return Err(AskError::Refused { iface: iface.to_string(), reason }),
        Ok(Outcome::Unsent(reason)) => return Err(AskError::Ended { iface: iface.to_string(), reason }),
        Err(_) if listener.withdraw(iface, tracked.id) => return Err(AskError::NotSent { iface: iface.to_string(), waited }),
        Err(_) => return Err(AskError::Stalled { iface: iface.to_string(), waited }),
    };

    // The devices' spread, and the wait past it.
    let window = Duration::from_millis(10).saturating_mul(u32::from(ask.response_delay)).saturating_add(ask.wait);
    std::thread::sleep(window);

    let mut asked = Asked { iface: iface.to_string(), xid, request: index, sent_ns, window, answers: Vec::new(), others: 0, lost: 0 };
    let (lost, frames) = listener.read_from(index + 1);
    asked.lost = lost;
    for (i, k) in frames {
        if k.sent || k.frame.iface != iface || k.frame.is_cut() {
            continue;
        }
        let Ok(e) = ether::parse(&k.frame.bytes) else { continue };
        if e.ethertype != ETHERTYPE_PROFINET || e.payload.get(..2) != Some(&dcp::FRAME_ID_IDENTIFY_RES.to_be_bytes()[..]) {
            continue;
        }
        let Ok(pdu) = dcp::decode(e.payload) else { continue };
        if pdu.service != Service::Identify || !pdu.service_type.is_response() {
            continue;
        }
        if pdu.xid == xid && e.dst == mac {
            asked.answers.push(Answer { mac: e.src, frame: i, at_ns: k.frame.at_ns, identity: DeviceIdentity::from_blocks(&pdu.blocks) });
        } else {
            asked.others += 1;
        }
    }
    Ok(asked)
}
