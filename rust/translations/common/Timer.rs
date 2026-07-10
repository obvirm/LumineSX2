// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! High-resolution monotonic timer.
//!
//! Idiomatic Rust translation of PCSX2's `Common::Timer` (Win32
//! `QueryPerformanceCounter` / POSIX `clock_gettime(CLOCK_MONOTONIC)`).
//!
//! All timing is anchored to a process-lifetime [`Instant`] captured at
//! first use of [`get_time_since_start`], so callers see nanosecond
//! resolution relative to program start. A [`PerformanceTimer`] is also
//! provided for callers that need raw counter values / ticks-per-second
//! in the same shape as the C++ API.

use std::time::{Duration, Instant};

/// Process-start anchor, lazily initialised on first access.
///
/// `Instant` is monotonic, so subtracting it from `Instant::now()` always
/// yields a non-negative duration, regardless of wall-clock changes.
static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();

/// Returns the duration elapsed since program start.
///
/// Subsequent calls share the same anchor (the first `Instant` captured
/// the first time this function runs), so a value of zero is only
/// possible in the (vanishingly unlikely) case where the call returns
/// before it has been able to store its anchor.
pub fn get_time_since_start() -> Duration {
    let start = START.get_or_init(Instant::now);
    start.elapsed()
}

/// High-resolution performance counter, mirroring the shape of the C++
/// `Common::Timer` value API.
///
/// Internally backed by [`Instant`]; the "ticks per second" reported is
/// always `1_000_000_000` because we surface nanoseconds directly. The
/// counter value is the number of nanoseconds elapsed since the
/// process-start anchor returned by [`get_time_since_start`].
pub struct PerformanceTimer;

impl PerformanceTimer {
    /// Number of counter ticks per second (always 1e9, since one tick
    /// is one nanosecond).
    pub fn get_ticks_per_second() -> u64 {
        1_000_000_000
    }

    /// Current counter value, in nanoseconds since the process-start
    /// anchor. Matches the C++ `Timer::GetCurrentValue()` semantics for
    /// the POSIX branch (`tv_sec * 1e9 + tv_nsec`) and is
    /// indistinguishable in practice from the Win32
    /// `QueryPerformanceCounter` branch after conversion.
    pub fn now() -> u64 {
        // `Instant::elapsed` saturates at zero if `now` is somehow
        // before the anchor; clamp to zero explicitly so the return
        // value is non-negative.
        get_time_since_start().as_nanos().min(u64::MAX as u128) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ticks_per_second_is_one_billion() {
        assert_eq!(PerformanceTimer::get_ticks_per_second(), 1_000_000_000);
    }

    #[test]
    fn now_is_non_decreasing() {
        let a = PerformanceTimer::now();
        // Busy-spin briefly to ensure the counter advances on fast
        // machines where the resolution is coarser than one tick.
        for _ in 0..1000 {
            let _ = PerformanceTimer::now();
        }
        let b = PerformanceTimer::now();
        assert!(b >= a, "now() must be monotonically non-decreasing");
    }

    #[test]
    fn time_since_start_matches_now() {
        let d = get_time_since_start();
        let n = PerformanceTimer::now();
        // `n` is `d` truncated to u64 ns; allow a 1-second skew window
        // to keep the test robust against future resolution changes.
        let diff = if d >= Duration::from_nanos(n) {
            (d - Duration::from_nanos(n)).as_nanos()
        } else {
            (Duration::from_nanos(n) - d).as_nanos()
        };
        assert!(diff < 1_000_000_000, "now() and time_since_start() diverge");
    }
}
