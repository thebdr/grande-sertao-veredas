//! The runtime's monotonic clock: time since the task started, and a precise wait until a deadline.
use std::time::{Duration, Instant};

/// Time since the runtime started. A spin margin turns the OS sleep into a precise wait: sleep until shortly
/// before the deadline, then spin. Without a real-time kernel this gives cheap precision; on Linux with
/// `SCHED_FIFO` the same loop runs real-time.
#[derive(Clone, Copy, Debug)]
pub struct MonoClock {
    start: Instant,
    spin: Duration,
}

impl Default for MonoClock {
    fn default() -> Self {
        Self::new()
    }
}

impl MonoClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            spin: Duration::from_micros(300),
        }
    }

    pub fn now(&self) -> Duration {
        self.start.elapsed()
    }

    /// Waits until `deadline` (a time since start). Returns at once when the deadline has already passed.
    pub fn sleep_until(&self, deadline: Duration) {
        let remaining = deadline.saturating_sub(self.now());
        if remaining > self.spin {
            std::thread::sleep(remaining.saturating_sub(self.spin));
        }
        while self.now() < deadline {
            std::hint::spin_loop();
        }
    }
}
