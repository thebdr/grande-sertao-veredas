//! The memory hub: one Ethernet segment in memory, for the tests that need no real link. Every
//! port hears what every other port sends (a hub, not a switch: what a frame's destination says is
//! the hearer's to judge), stamped with the hub's clock and the hearing port's name, as a card
//! stamps a frame with its own interface. A port never hears its own frames.
//!
//! The clock is the test's: it starts at 0 and moves only when told, so a test's stamps are its
//! own. A port keeps at most `QUEUE` frames unread; past that the oldest is let go and counted, as
//! a card's ring drops what its reader does not take in time.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::{Frame, Wire, WireError, sendable};
use crate::profinet::mac::Mac;

/// The most frames a port keeps unread.
pub const QUEUE: usize = 4096;

/// One segment in memory. Cloning it gives another handle on the same segment.
#[derive(Clone, Debug, Default)]
pub struct Hub {
    inner: Arc<Mutex<Segment>>,
}

#[derive(Debug, Default)]
struct Segment {
    now_ns: u64,
    next_id: usize,
    ports: Vec<(usize, String, Arc<Inbox>)>,
}

#[derive(Debug, Default)]
struct Inbox {
    queue: Mutex<Queue>,
    arrived: Condvar,
}

#[derive(Debug, Default)]
struct Queue {
    frames: VecDeque<Frame>,
    dropped: u64,
}

/// A lock that a panic elsewhere left poisoned is taken all the same: the hub's state is a list
/// and queues, whole after every step.
fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Hub {
    pub fn new() -> Hub {
        Hub::default()
    }

    /// Set the clock that stamps every frame from now on.
    pub fn set_now(&self, now_ns: u64) {
        lock(&self.inner).now_ns = now_ns;
    }

    pub fn now(&self) -> u64 {
        lock(&self.inner).now_ns
    }

    /// A new port on the segment: an interface named `name` (two ports may share a name, as two
    /// hosts' cards do) with its own MAC. It leaves the segment when dropped.
    pub fn port(&self, name: impl Into<String>, mac: Mac) -> HubPort {
        let name = name.into();
        let inbox = Arc::new(Inbox::default());
        let mut segment = lock(&self.inner);
        let id = segment.next_id;
        segment.next_id += 1;
        segment.ports.push((id, name.clone(), Arc::clone(&inbox)));
        HubPort { id, name, mac, hub: Arc::clone(&self.inner), inbox }
    }

    /// The ports on the segment now.
    pub fn ports(&self) -> usize {
        lock(&self.inner).ports.len()
    }
}

/// One port of a hub: a wire.
#[derive(Debug)]
pub struct HubPort {
    id: usize,
    name: String,
    mac: Mac,
    hub: Arc<Mutex<Segment>>,
    inbox: Arc<Inbox>,
}

impl HubPort {
    /// The frames this port let go unread, its queue full.
    pub fn dropped(&self) -> u64 {
        lock(&self.inbox.queue).dropped
    }
}

impl Drop for HubPort {
    fn drop(&mut self) {
        lock(&self.hub).ports.retain(|(id, _, _)| *id != self.id);
    }
}

impl Wire for HubPort {
    fn name(&self) -> &str {
        &self.name
    }

    fn mac(&self) -> Option<Mac> {
        Some(self.mac)
    }

    fn send(&mut self, frame: &[u8]) -> Result<(), WireError> {
        sendable(frame)?;
        let segment = lock(&self.hub);
        for (id, name, inbox) in &segment.ports {
            if *id == self.id {
                continue;
            }
            let mut q = lock(&inbox.queue);
            if q.frames.len() == QUEUE {
                q.frames.pop_front();
                q.dropped += 1;
            }
            q.frames.push_back(Frame::new(segment.now_ns, name.clone(), frame.to_vec()));
            inbox.arrived.notify_all();
        }
        Ok(())
    }

    fn recv(&mut self, timeout: Duration) -> Result<Option<Frame>, WireError> {
        let deadline = Instant::now().checked_add(timeout);
        let mut q = lock(&self.inbox.queue);
        loop {
            if let Some(f) = q.frames.pop_front() {
                return Ok(Some(f));
            }
            let left = match deadline {
                None => Duration::MAX,
                Some(d) => {
                    let now = Instant::now();
                    if now >= d {
                        return Ok(None);
                    }
                    d - now
                }
            };
            q = self.inbox.arrived.wait_timeout(q, left).unwrap_or_else(PoisonError::into_inner).0;
        }
    }
}
