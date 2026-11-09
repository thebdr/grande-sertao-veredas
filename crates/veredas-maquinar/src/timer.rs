//! The host's timer resolution, held fine while the bench runs (`docs/questions.md` Q13, decided
//! 2026-11-09). A runtime's tick lands at the host's timer quantum: about 15.6 ms on Windows
//! unless the process asks for better, which made the bench's 20 ms tick land every 31 ms there.
//! The bench asks: `timeBeginPeriod(1)` while any of its parts runs, paired with `timeEndPeriod(1)`
//! when the guard drops; and, since Windows 11 ignores that request from a process without a
//! foreground window unless it opts out of power throttling's timer coarsening, the process opts
//! out first (`SetProcessInformation` with `PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION`; the
//! request alone still ticked every 30 ms here, both together at 20). So the tick is 20 ms on both
//! platforms and the clauses can state one figure. Elsewhere the guard is a no-op.

/// A guard holding the host's timer at a millisecond; drop it to let go. Guards nest: the
/// resolution is asked once per guard and released once per guard, as the host counts them.
#[must_use = "the timer resolution is held only while the guard lives"]
pub struct FineTimer(());

impl FineTimer {
    /// Hold the host's timer at a millisecond until the guard drops.
    pub fn hold() -> FineTimer {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Media::timeBeginPeriod;
            use windows_sys::Win32::System::Threading::{
                GetCurrentProcess, ProcessPowerThrottling, SetProcessInformation, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
            };
            // The opt-out is a process setting, idempotent, and left in place: the bench process
            // is the bench's own. StateMask 0 with the bit in ControlMask means "do not coarsen".
            let state = PROCESS_POWER_THROTTLING_STATE {
                Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
                ControlMask: PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION,
                StateMask: 0,
            };
            // SAFETY: plain calls into kernel32 and winmm with constants and a stack struct of the
            // documented size; the pairing with `timeEndPeriod` is the guard's Drop.
            unsafe {
                SetProcessInformation(
                    GetCurrentProcess(),
                    ProcessPowerThrottling,
                    (&state as *const PROCESS_POWER_THROTTLING_STATE).cast(),
                    std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
                );
                timeBeginPeriod(1);
            }
        }
        FineTimer(())
    }
}

impl Drop for FineTimer {
    fn drop(&mut self) {
        #[cfg(windows)]
        {
            // SAFETY: the matching call for the `timeBeginPeriod(1)` of `hold`.
            unsafe {
                windows_sys::Win32::Media::timeEndPeriod(1);
            }
        }
    }
}
