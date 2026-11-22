//! One cyclic task: the scan loop with the process image, the watchdog and the statistics.
use crate::clock::MonoClock;
use orcl_std::{Program, Scan};
use std::time::Duration;

/// The task's connection to the plant: real I/O, a fieldbus, or a simulation. `read_inputs` is called once at
/// the start of every scan. `write_outputs` is called once after it, with the program's outputs or, once
/// faulted, its safe outputs.
pub trait Io<I, O> {
    fn read_inputs(&mut self, now: Duration) -> I;
    fn write_outputs(&mut self, now: Duration, outputs: &O);
}

#[derive(Clone, Debug)]
pub struct TaskConfig {
    pub name: &'static str,
    pub period: Duration,
    /// How many consecutive overrunning cycles the watchdog tolerates before it faults the task.
    pub overrun_limit: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CycleStats {
    pub cycles: u64,
    pub overruns: u64,
    pub skipped: u64,
    pub max_jitter: Duration,
    pub total_jitter: Duration,
    pub max_exec: Duration,
    pub total_exec: Duration,
}

impl CycleStats {
    pub fn mean_jitter(&self) -> Duration {
        mean(self.total_jitter, self.cycles)
    }

    pub fn mean_exec(&self) -> Duration {
        mean(self.total_exec, self.cycles)
    }

    fn record(&mut self, jitter: Duration, exec: Duration) {
        self.cycles = self.cycles.saturating_add(1);
        self.max_jitter = self.max_jitter.max(jitter);
        self.total_jitter = self.total_jitter.saturating_add(jitter);
        self.max_exec = self.max_exec.max(exec);
        self.total_exec = self.total_exec.saturating_add(exec);
    }
}

fn mean(total: Duration, count: u64) -> Duration {
    let nanos = total.as_nanos().checked_div(u128::from(count)).unwrap_or(0);
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    Running,
    Faulted { at: Duration, reason: &'static str },
}

/// What the monitor sees after every scan.
pub struct Report<'a, P: Program> {
    pub scan: Scan,
    pub program: &'a P,
    pub inputs: &'a P::Inputs,
    pub outputs: &'a P::Outputs,
    pub stats: &'a CycleStats,
    pub state: RunState,
}

/// The deadline of cycle `k`: `k * period` from the start, saturating.
fn deadline(period: Duration, cycle: u64) -> Duration {
    let nanos = period.as_nanos().saturating_mul(u128::from(cycle));
    Duration::from_nanos(u64::try_from(nanos).unwrap_or(u64::MAX))
}

/// Runs `program` as a cyclic task for `duration`, calling `monitor` after every scan. Returns the statistics
/// and the final state.
pub fn run<P, D, M>(
    config: &TaskConfig,
    program: &mut P,
    io: &mut D,
    duration: Duration,
    mut monitor: M,
) -> (CycleStats, RunState)
where
    P: Program,
    D: Io<P::Inputs, P::Outputs>,
    M: FnMut(&Report<'_, P>),
{
    let clock = MonoClock::new();
    let period = config.period.max(Duration::from_micros(100));
    let mut stats = CycleStats::default();
    let mut state = RunState::Running;
    let mut outputs = P::Outputs::default();
    let mut consecutive_overruns: u32 = 0;
    let mut cycle: u64 = 0;
    loop {
        let due = deadline(period, cycle);
        if due >= duration {
            break;
        }
        clock.sleep_until(due);
        let start = clock.now();
        let scan = Scan {
            now: start,
            period,
            cycle,
        };
        let inputs = io.read_inputs(start);
        if state == RunState::Running {
            program.scan(&scan, &inputs, &mut outputs);
        } else {
            outputs = program.safe_outputs();
        }
        io.write_outputs(start, &outputs);
        let exec = clock.now().saturating_sub(start);
        stats.record(start.saturating_sub(due), exec);

        if exec > period {
            stats.overruns = stats.overruns.saturating_add(1);
            consecutive_overruns = consecutive_overruns.saturating_add(1);
            if consecutive_overruns > config.overrun_limit && state == RunState::Running {
                state = RunState::Faulted {
                    at: start,
                    reason: "watchdog: the cycle overran its period",
                };
                outputs = program.safe_outputs();
                io.write_outputs(clock.now(), &outputs);
            }
        } else {
            consecutive_overruns = 0;
        }

        monitor(&Report {
            scan,
            program: &*program,
            inputs: &inputs,
            outputs: &outputs,
            stats: &stats,
            state,
        });

        // The next cycle is the next one still due: a late scan skips the deadlines it missed (counted).
        let next = cycle.saturating_add(1);
        let now_cycle = u64::try_from(
            clock
                .now()
                .as_nanos()
                .checked_div(period.as_nanos())
                .unwrap_or(0),
        )
        .unwrap_or(u64::MAX);
        if now_cycle >= next {
            stats.skipped = stats
                .skipped
                .saturating_add(now_cycle.saturating_sub(next).saturating_add(1));
            cycle = now_cycle.saturating_add(1);
        } else {
            cycle = next;
        }
    }
    (stats, state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Count;

    impl Program for Count {
        type Inputs = u32;
        type Outputs = u32;

        fn scan(&mut self, _scan: &Scan, inputs: &u32, outputs: &mut u32) {
            *outputs = inputs.saturating_add(1);
        }

        fn safe_outputs(&self) -> u32 {
            0
        }
    }

    struct Loop {
        value: u32,
        delay: Duration,
    }

    impl Io<u32, u32> for Loop {
        fn read_inputs(&mut self, _now: Duration) -> u32 {
            std::thread::sleep(self.delay);
            self.value
        }

        fn write_outputs(&mut self, _now: Duration, outputs: &u32) {
            self.value = *outputs;
        }
    }

    #[test]
    fn runs_one_scan_per_period_through_the_process_image() {
        let config = TaskConfig {
            name: "t",
            period: Duration::from_millis(5),
            overrun_limit: 3,
        };
        let mut io = Loop {
            value: 0,
            delay: Duration::ZERO,
        };
        let (stats, state) = run(
            &config,
            &mut Count,
            &mut io,
            Duration::from_millis(200),
            |_| {},
        );
        assert_eq!(state, RunState::Running);
        assert!(
            (30..=40).contains(&stats.cycles),
            "about 40 cycles of 5 ms in 200 ms: {stats:?}"
        );
        assert_eq!(
            u64::from(io.value),
            stats.cycles,
            "each scan saw the outputs of the one before"
        );
    }

    #[test]
    fn the_watchdog_faults_into_the_safe_outputs() {
        let config = TaskConfig {
            name: "t",
            period: Duration::from_millis(2),
            overrun_limit: 2,
        };
        let mut io = Loop {
            value: 0,
            delay: Duration::from_millis(5),
        };
        let (stats, state) = run(
            &config,
            &mut Count,
            &mut io,
            Duration::from_millis(100),
            |_| {},
        );
        assert!(matches!(state, RunState::Faulted { .. }), "{state:?}");
        assert!(stats.overruns >= 3 && stats.skipped > 0, "{stats:?}");
        assert_eq!(io.value, 0, "a faulted task applies the safe outputs");
    }
}
