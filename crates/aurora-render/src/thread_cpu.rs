//! CPU time of the calling thread, in the processor's cycles
//! (AURORA_PROFILE): with the clock, it tells a step that worked from one
//! that waited — for a lock, or for a core behind another thread. Known on
//! Windows only; elsewhere the counts are 0 and nothing is reported.

use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Cycles the calling thread has run so far (0 where unknown).
pub fn cycles() -> u64 {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::{Threading::GetCurrentThread, WindowsProgramming::QueryThreadCycleTime};
        let mut cycles = 0u64;
        // SAFETY: `GetCurrentThread` returns the calling thread's
        // pseudo-handle, always valid for it; the call writes one u64.
        unsafe {
            QueryThreadCycleTime(GetCurrentThread(), &mut cycles);
        }
        cycles
    }
    #[cfg(not(windows))]
    0
}

/// Cycles of a millisecond on the processor (0 where unknown), measured
/// once: the fastest of a few short busy loops, so that one cut short by
/// another thread does not lower the rate.
pub fn cycles_per_ms() -> f64 {
    static RATE: OnceLock<f64> = OnceLock::new();
    *RATE.get_or_init(|| {
        let mut best = 0.0f64;
        for _ in 0..8 {
            let (t0, c0) = (Instant::now(), cycles());
            while t0.elapsed() < Duration::from_micros(250) {
                std::hint::spin_loop();
            }
            let (ms, ran) = (t0.elapsed().as_secs_f64() * 1000.0, cycles().wrapping_sub(c0));
            best = best.max(ran as f64 / ms);
        }
        best
    })
}

/// Time off the processor (ms) of a step that lasted `ms` and ran `cycles`;
/// 0 where the thread's CPU time is unknown.
pub fn waited_ms(ms: f32, cycles: u64) -> f32 {
    let rate = cycles_per_ms();
    if rate > 0.0 {
        (ms - (cycles as f64 / rate) as f32).max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_loop_is_on_the_processor_and_a_sleep_is_not() {
        if cycles_per_ms() == 0.0 {
            // unknown on this platform: nothing is ever reported as waited
            assert_eq!(waited_ms(5.0, 0), 0.0);
            return;
        }
        // On a loaded machine another thread takes the core in the middle of
        // a loop, for longer than the loop lasts (seen on CI: 34 ms out of
        // 37), and that loop did wait: the claim is for a loop left alone.
        // Short ones are tried until one is, a bounded number of times; half
        // on the processor is enough to tell it from a sleep.
        let mut least = (f32::MAX, 0.0, 0);
        for _ in 0..40 {
            let (t0, c0) = (Instant::now(), cycles());
            while t0.elapsed() < Duration::from_millis(5) {
                std::hint::spin_loop();
            }
            let (busy_ms, busy) = (t0.elapsed().as_secs_f32() * 1000.0, cycles() - c0);
            let waited = waited_ms(busy_ms, busy) / busy_ms;
            if waited < least.0 {
                least = (waited, busy_ms, busy);
            }
            if waited < 0.5 {
                break;
            }
        }
        // a sleep costs next to no cycles, however long the machine makes it
        // last
        let (t1, c1) = (Instant::now(), cycles());
        std::thread::sleep(Duration::from_millis(20));
        let (sleep_ms, slept) = (t1.elapsed().as_secs_f32() * 1000.0, cycles() - c1);
        let (waited, busy_ms, busy) = least;
        assert!(waited < 0.5, "{busy_ms} ms, {busy} cycles at best");
        assert!(waited_ms(sleep_ms, slept) > sleep_ms * 0.9, "{sleep_ms} ms, {slept} cycles");
    }
}
