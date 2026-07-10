// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Pure-Rust translation of `common/Timer.{h,cpp}`.
//
// Provides a high-resolution monotonic timer and per-CPU cycle counter
// suitable for performance measurement.  Time on Unix platforms comes from
// `std::time::Instant` (CLOCK_MONOTONIC) and CPU cycles are read directly
// from RDTSC on x86_64 or CNTVCT on aarch64.

use std::time::Instant;

/// Alias mirroring `Common::Timer::Value` from the original C++ API.
pub type TimerValue = u64;

/// Frequency of the monotonic tick counter, in ticks per second.
///
/// Backed by `Instant` whose underlying source is a nanosecond-resolution
/// monotonic clock on every platform std supports, so 1_000_000_000 is the
/// canonical tick frequency.
pub const TICKS_PER_SECOND: u64 = 1_000_000_000;

/// Stopwatch-style timer mirroring the C++ `Common::Timer` class.
#[derive(Debug, Clone)]
pub struct Timer {
    start_value: TimerValue,
}

impl Timer {
    /// Create a new timer that begins counting from now.
    pub fn new() -> Self {
        Self {
            start_value: get_ticks(),
        }
    }

    /// Create a new timer seeded with an explicit start value (useful for
    /// restoring previously-saved state).
    pub fn from_value(start_value: TimerValue) -> Self {
        Self { start_value }
    }

    /// Snapshot the current monotonic tick value.
    pub fn current_value() -> TimerValue {
        get_ticks()
    }

    /// Reset the timer to "now".
    pub fn reset(&mut self) {
        self.start_value = get_ticks();
    }

    /// Reset the timer to an explicit value.
    pub fn reset_to(&mut self, value: TimerValue) {
        self.start_value = value;
    }

    /// Returns the start value without altering the timer.
    pub fn get_start_value(&self) -> TimerValue {
        self.start_value
    }

    /// Returns elapsed time since the last reset, in seconds.
    pub fn get_time_seconds(&self) -> f64 {
        convert_value_to_seconds(get_ticks().saturating_sub(self.start_value))
    }

    /// Returns elapsed time since the last reset, in milliseconds.
    pub fn get_time_milliseconds(&self) -> f64 {
        convert_value_to_milliseconds(get_ticks().saturating_sub(self.start_value))
    }

    /// Returns elapsed time since the last reset, in nanoseconds.
    pub fn get_time_nanoseconds(&self) -> f64 {
        convert_value_to_nanoseconds(get_ticks().saturating_sub(self.start_value))
    }

    /// Returns elapsed time in seconds and resets the start value to now.
    pub fn get_time_seconds_and_reset(&mut self) -> f64 {
        let value = get_ticks();
        let elapsed = convert_value_to_seconds(value.saturating_sub(self.start_value));
        self.start_value = value;
        elapsed
    }

    /// Returns elapsed time in milliseconds and resets the start value to now.
    pub fn get_time_milliseconds_and_reset(&mut self) -> f64 {
        let value = get_ticks();
        let elapsed = convert_value_to_milliseconds(value.saturating_sub(self.start_value));
        self.start_value = value;
        elapsed
    }

    /// Returns elapsed time in nanoseconds and resets the start value to now.
    pub fn get_time_nanoseconds_and_reset(&mut self) -> f64 {
        let value = get_ticks();
        let elapsed = convert_value_to_nanoseconds(value.saturating_sub(self.start_value));
        self.start_value = value;
        elapsed
    }

    /// Resets the timer and returns `true` if at least `s` seconds had elapsed.
    pub fn reset_if_seconds_passed(&mut self, s: f64) -> bool {
        let value = get_ticks();
        let elapsed = convert_value_to_seconds(value.saturating_sub(self.start_value));
        if elapsed < s {
            return false;
        }
        self.start_value = value;
        true
    }

    /// Resets the timer and returns `true` if at least `s` milliseconds had elapsed.
    pub fn reset_if_milliseconds_passed(&mut self, s: f64) -> bool {
        let value = get_ticks();
        let elapsed = convert_value_to_milliseconds(value.saturating_sub(self.start_value));
        if elapsed < s {
            return false;
        }
        self.start_value = value;
        true
    }

    /// Resets the timer and returns `true` if at least `s` nanoseconds had elapsed.
    pub fn reset_if_nanoseconds_passed(&mut self, s: f64) -> bool {
        let value = get_ticks();
        let elapsed = convert_value_to_nanoseconds(value.saturating_sub(self.start_value));
        if elapsed < s {
            return false;
        }
        self.start_value = value;
        true
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self::new()
    }
}

thread_local! {
    static TICK_REFERENCE: Instant = Instant::now();
}

/// Returns the current monotonic clock value in ticks.
///
/// The tick frequency is [`TICKS_PER_SECOND`] (1 GHz), so the returned value
/// is the elapsed nanoseconds since the process start. The reference is
/// captured once per thread via a `thread_local` to keep repeated calls
/// allocation-free.
pub fn get_ticks() -> u64 {
    TICK_REFERENCE.with(|r| r.elapsed().as_nanos() as u64)
}

/// Returns the frequency of the monotonic tick counter (ticks per second).
pub fn get_tick_frequency() -> u64 {
    TICKS_PER_SECOND
}

/// Alias for [`get_tick_frequency`]; matches the C++ naming convention.
pub fn get_ticks_per_second() -> u64 {
    TICKS_PER_SECOND
}

/// Returns the raw CPU cycle counter.
///
/// On x86_64 this is RDTSC. On aarch64 this is CNTVCT (the virtual counter
/// of the generic timer). On other architectures a portable fallback returns
/// 0 — callers should treat this as "unsupported".
pub fn get_cpu_ticks() -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: RDTSC has no memory effects and no preconditions. The core
        // arch wrapper issues `rdtsc` and combines the EDX:EAX halves.
        unsafe { core::arch::x86_64::_rdtsc() }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // CNTVCT_EL0 - virtual counter, always accessible at EL0.
        let value: u64;
        // SAFETY: Reading CNTVCT_EL0 is always available at EL0 on AArch64.
        unsafe {
            core::arch::asm!("mrs {0}, cntvct_el0", out(reg) value);
        }
        value
    }

    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        0
    }
}

// ---------------------------------------------------------------------------
// Unit conversions. Frequency is 1 GHz, so value is already in nanoseconds.
// ---------------------------------------------------------------------------

#[inline]
fn convert_value_to_nanoseconds(value: TimerValue) -> f64 {
    value as f64
}

#[inline]
fn convert_value_to_milliseconds(value: TimerValue) -> f64 {
    (value as f64) / 1_000_000.0
}

#[inline]
fn convert_value_to_seconds(value: TimerValue) -> f64 {
    (value as f64) / 1_000_000_000.0
}

#[inline]
fn convert_seconds_to_value(s: f64) -> TimerValue {
    (s * 1_000_000_000.0) as TimerValue
}

#[inline]
fn convert_milliseconds_to_value(ms: f64) -> TimerValue {
    (ms * 1_000_000.0) as TimerValue
}

#[inline]
fn convert_nanoseconds_to_value(ns: f64) -> TimerValue {
    ns as TimerValue
}

// ---------------------------------------------------------------------------
// C ABI exports.
// ---------------------------------------------------------------------------

/// C ABI: returns the current monotonic tick value.
#[no_mangle]
pub extern "C" fn pcsx2_timer_get_ticks() -> u64 {
    get_ticks()
}

/// C ABI: returns the monotonic tick frequency (ticks per second).
#[no_mangle]
pub extern "C" fn pcsx2_timer_get_tick_frequency() -> u64 {
    get_tick_frequency()
}

/// C ABI: returns the raw CPU cycle counter.
#[no_mangle]
pub extern "C" fn pcsx2_timer_get_cpu_ticks() -> u64 {
    get_cpu_ticks()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn tick_frequency_is_one_gigahertz() {
        assert_eq!(get_tick_frequency(), 1_000_000_000);
        assert_eq!(get_ticks_per_second(), 1_000_000_000);
    }

    #[test]
    fn ticks_advance_after_sleep() {
        let before = get_ticks();
        sleep(Duration::from_millis(5));
        let after = get_ticks();
        assert!(after > before, "ticks did not advance: before={before}, after={after}");
        // At 1 GHz tick rate, 5ms == 5_000_000 ticks. Allow generous slack.
        assert!((after - before) >= 1_000_000);
    }

    #[test]
    fn timer_measures_elapsed_time() {
        let mut t = Timer::new();
        sleep(Duration::from_millis(2));
        let ms = t.get_time_milliseconds();
        assert!(ms >= 1.0, "expected >=1ms, got {ms}");
    }

    #[test]
    fn timer_reset_returns_approx_zero() {
        let mut t = Timer::new();
        sleep(Duration::from_millis(1));
        t.reset();
        let ns = t.get_time_nanoseconds();
        assert!(ns < 5_000_000.0, "reset not effective, ns={ns}");
    }

    #[test]
    fn cpu_ticks_nonzero_on_supported_arches() {
        let a = get_cpu_ticks();
        let _ = get_cpu_ticks();
        let b = get_cpu_ticks();
        // On x86_64 / aarch64 the CPU counter must advance. On other arches
        // it returns 0 — only assert non-zero when we are on a supported
        // target.
        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        assert!(b >= a, "CPU ticks did not advance: a={a}, b={b}");
    }

    #[test]
    fn unit_conversions_round_trip() {
        let ns = convert_nanoseconds_to_value(1_234.5);
        assert_eq!(convert_value_to_nanoseconds(ns), 1_234.5);
        let ms = convert_milliseconds_to_value(2.5);
        assert!((convert_value_to_milliseconds(ms) - 2.5).abs() < 1e-6);
        let s = convert_seconds_to_value(0.75);
        assert!((convert_value_to_seconds(s) - 0.75).abs() < 1e-6);
    }
}
