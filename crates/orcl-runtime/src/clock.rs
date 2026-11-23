//! The clocks a task runs on: the time a scan reads and the deadlines a task waits for. The engine reads no
//! clock of its own; its host hands one in, so the same scan runs on a target's monotonic clock and on virtual
//! time in a test or a simulation.
use core::cell::Cell;
use core::time::Duration;
use std::rc::Rc;
use std::time::Instant;

/// The program clock as a task sees it: time since the task started, and a wait until a deadline on it.
pub trait Clock {
    /// Time since the clock started.
    fn now(&self) -> Duration;

    /// Waits until `deadline` (a time since start). Returns at once when the deadline is not in the future, and
    /// never before it.
    fn sleep_until(&mut self, deadline: Duration);
}

/// A target's clock: monotonic time since it was made. A spin margin turns the operating system's sleep into a
/// precise wait: sleep until shortly before the deadline, then spin. Real-time scheduling, if the host sets it
/// up, makes the same loop real time; the clock itself asks for nothing.
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
    /// A clock starting now, with a spin margin of 300 µs.
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
            spin: Duration::from_micros(300),
        }
    }
}

impl Clock for MonoClock {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }

    fn sleep_until(&mut self, deadline: Duration) {
        let remaining = deadline.saturating_sub(self.now());
        if remaining > self.spin {
            std::thread::sleep(remaining.saturating_sub(self.spin));
        }
        while self.now() < deadline {
            core::hint::spin_loop();
        }
    }
}

/// Virtual time, for a test or a simulation: it stands still until something moves it. Waiting for a deadline
/// moves it there at once, and [`VirtualClock::advance`] moves it forward, which is how a simulated plant or a
/// test stands for the time a scan takes. Clones share one time, so the task's host and the plant read the same
/// clock. Hours of plant time run in milliseconds.
#[derive(Clone, Debug, Default)]
pub struct VirtualClock {
    now: Rc<Cell<Duration>>,
}

impl VirtualClock {
    /// A clock standing at zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Moves the clock forward by `by`, saturating.
    pub fn advance(&self, by: Duration) {
        self.now.set(self.now.get().saturating_add(by));
    }
}

impl Clock for VirtualClock {
    fn now(&self) -> Duration {
        self.now.get()
    }

    fn sleep_until(&mut self, deadline: Duration) {
        if deadline > self.now.get() {
            self.now.set(deadline);
        }
    }
}
