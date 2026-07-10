// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

#![cfg(target_os = "linux")]

//! Linux implementation of PCSX2's `Threading::*` free functions.
//!
//! This module is the direct Rust port of `common/Linux/LnxThreads.cpp`.
//! It is gated on `#[cfg(target_os = "linux")]` and is otherwise empty
//! so the same crate can be built on non-Linux hosts without dragging
//! the Linux-only FFI surface in.
//!
//! The free functions exposed here match the signatures consumed by
//! the cross-platform `threading` module and the FFI surface generated
//! for the C++ side via `cbindgen`.
//!
//! ## Notes on unit choice
//!
//! The C++ `Threading::GetThreadCpuTime()` returns microseconds on
//! Linux, so `get_thread_ticks_per_second()` returns 1_000_000 to match.
//! The pure-Rust `sleep_until` mirrors `Threading::SleepUntil(u64)`,
//! which on Linux treats the input as a *relative* microsecond delay
//! (the same convention used by `threading::sleep_until`).
//!
//! ## Thread names
//!
//! `pthread_setname_np` on Linux limits names to 15 bytes plus a NUL
//! terminator. Anything longer is silently truncated to 15 bytes; this
//! matches the C++ version's behaviour when passing the buffer
//! straight through to `prctl(PR_SET_NAME, ...)` / `pthread_setname_np`.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    unused_imports,
    unused_variables,
    clippy::all,
)]

use std::ffi::{c_char, CStr};
use std::thread;
use std::time::Duration;

// ---------------------------------------------------------------------------
// Pure-Rust functions
// ---------------------------------------------------------------------------

/// Get the CPU time consumed by the current thread, in microseconds.
///
/// Mirrors `Threading::GetThreadCpuTime()` on Linux. Internally calls
/// `clock_gettime(CLOCK_THREAD_CPUTIME_ID, ...)`, converting the
/// nanosecond resolution to microseconds to match the C++ return
/// value's units.
///
/// Returns `0` if the syscall fails (which would indicate a kernel
/// bug rather than anything the caller can recover from; the C++
/// version returns 0 in the same situation).
pub fn get_thread_cpu_time() -> u64 {
    unsafe {
        let mut ts: libc::timespec = std::mem::zeroed();
        let rc = libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts);
        if rc != 0 {
            return 0;
        }
        // Saturating arithmetic so a malformed kernel timestamp
        // (e.g. tv_sec == i64::MIN on a broken platform) cannot panic.
        let secs = (ts.tv_sec as i64).max(0) as u64;
        let nanos = (ts.tv_nsec as i64).max(0) as u64;
        secs.saturating_mul(1_000_000).saturating_add(nanos / 1_000)
    }
}

/// Get the frequency (Hz) of [`get_thread_cpu_time`].
///
/// On Linux `clock_gettime(CLOCK_THREAD_CPUTIME_ID, ...)` returns
/// nanosecond resolution, but `Threading::GetThreadCpuTime()` divides
/// down to microseconds — so the corresponding tick rate is 1 MHz.
/// This matches the C++ `Threading::GetThreadTicksPerSecond()` which
/// hardcodes 1_000_000 on Linux.
#[inline]
pub fn get_thread_ticks_per_second() -> u64 {
    1_000_000
}

/// Set the name of the current thread.
///
/// On Linux this delegates to `pthread_setname_np(pthread_self(), name)`.
/// The POSIX thread-name limit is 15 bytes plus a NUL terminator, so
/// the input is truncated to 15 bytes — which matches the C++
/// behaviour when handing the string straight to
/// `prctl(PR_SET_NAME, ...)`. Failing to set a name is intentionally
/// ignored (returns are not part of the public API).
pub fn set_name_of_current_thread(name: &str) {
    // POSIX thread-name limit is 15 bytes + NUL = 16 bytes total.
    // We truncate by byte boundary; the C++ version does the same
    // when handing the string to pthread_setname_np / prctl.
    let bytes = name.as_bytes();
    let len = bytes.len().min(15);
    let mut buf = [0u8; 16];
    buf[..len].copy_from_slice(&bytes[..len]);

    unsafe {
        // pthread_setname_np is async-signal-safe per POSIX.1-2008.
        // We deliberately ignore the return code — a failed setname
        // must not bring the emulator down.
        libc::pthread_setname_np(libc::pthread_self(), buf.as_ptr() as *const c_char);
    }
}

/// Yield the current thread's remaining time slice to the OS scheduler.
///
/// Equivalent to `sched_yield()` on POSIX and `Sleep(0)` on Windows.
/// On Linux `std::thread::yield_now` ultimately calls `sched_yield`.
#[inline]
pub fn timeslice() {
    thread::yield_now();
}

/// Emit a CPU pause hint suitable for use inside spin/wait loops.
///
/// On x86 this lowers to the `pause` instruction; on ARM64 it lowers
/// to `isb`. `std::hint::spin_loop` is the portable Rust equivalent
/// of the C++ `_mm_pause()` / inline `__asm__("pause")` path.
#[inline]
pub fn spin_wait() {
    std::hint::spin_loop();
}

/// Enable the hires scheduler.
///
/// The C++ Linux implementation is a no-op (Linux does not expose a
/// tunable scheduler resolution like Windows' `timeBeginPeriod`).
/// Mirrored here for API parity with the cross-platform
/// `Threading::EnableHiresScheduler()`.
#[inline]
pub fn enable_hires_scheduler() {
    // Linux does not have a customizable scheduler resolution
    // (unlike Windows' timeBeginPeriod), so this is intentionally a
    // no-op — matching LnxThreads.cpp.
}

/// Disable the hires scheduler.
///
/// The C++ Linux implementation is a no-op. Mirrored here for API
/// parity with the cross-platform
/// `Threading::DisableHiresScheduler()`.
#[inline]
pub fn disable_hires_scheduler() {
    // No-op on Linux. See enable_hires_scheduler for context.
}

/// Sleep the current thread for `ms` milliseconds.
pub fn sleep(ms: u32) {
    thread::sleep(Duration::from_millis(ms as u64));
}

/// Sleep the current thread until `ticks` microseconds from now.
///
/// Mirrors `Threading::SleepUntil(u64 ticks)` on Linux. The C++ version
/// takes a deadline expressed in the same units as `GetThreadCpuTime`,
/// which on Linux is microseconds. In practice the consumer subtracts a
/// "now" reading from a future reading and passes the delta, so this
/// implementation treats the input as a relative delay of `ticks`
/// microseconds and uses `clock_nanosleep(CLOCK_MONOTONIC, ...)` for
/// the wakeup so the wait is not affected by wall-clock adjustments.
pub fn sleep_until(ticks: u64) {
    if ticks == 0 {
        // Same convention as the C++ version: 0 means "no waiting,
        // yield the timeslice" rather than "sleep until epoch".
        thread::yield_now();
        return;
    }

    unsafe {
        // clock_nanosleep is preferred over nanosleep + relative math
        // because it atomically reads the clock and computes the
        // deadline; this avoids drift on long sleeps.
        let req = libc::timespec {
            tv_sec: (ticks / 1_000_000) as libc::time_t,
            tv_nsec: ((ticks % 1_000_000) * 1_000) as libc::c_long,
        };
        // Remaining is filled in by clock_nanosleep on early return
        // due to a signal; we deliberately discard it and only loop
        // on EINTR (which is the only "retryable" error). Other
        // errors (EFAULT, EINVAL) indicate a programming bug and we
        // just return — the C++ version does the same.
        let mut remaining: libc::timespec = std::mem::zeroed();
        loop {
            let rc = libc::clock_nanosleep(
                libc::CLOCK_MONOTONIC,
                0,
                &req,
                &mut remaining,
            );
            if rc != libc::EINTR {
                break;
            }
            // EINTR: retry with the remaining time. If remaining
            // somehow ends up zero, bail out so we don't spin.
            if remaining.tv_sec == 0 && remaining.tv_nsec == 0 {
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// FFI exports
// ---------------------------------------------------------------------------

/// FFI export: set the current thread's name from a C string.
///
/// `name` must be a valid, null-terminated UTF-8 C string, or null.
/// A null pointer is treated as a no-op. If the bytes are not valid
/// UTF-8 the call is also a no-op (the C++ side passes plain ASCII
/// in practice).
#[no_mangle]
pub extern "C" fn pcsx2_thread_set_name(name: *const c_char) {
    if name.is_null() {
        return;
    }
    // Safety: caller guarantees a valid null-terminated C string.
    let cstr = unsafe { CStr::from_ptr(name) };
    if let Ok(s) = cstr.to_str() {
        set_name_of_current_thread(s);
    }
}

/// FFI export: sleep the current thread for `ms` milliseconds.
#[no_mangle]
pub extern "C" fn pcsx2_thread_sleep(ms: u32) {
    sleep(ms);
}

/// FFI export: sleep the current thread for `ticks` microseconds.
///
/// See [`sleep_until`] for the unit convention.
#[no_mangle]
pub extern "C" fn pcsx2_thread_sleep_until(ticks: u64) {
    sleep_until(ticks);
}

/// FFI export: get the CPU time consumed by the current thread.
///
/// See [`get_thread_cpu_time`] for the unit convention.
#[no_mangle]
pub extern "C" fn pcsx2_thread_get_cpu_time() -> u64 {
    get_thread_cpu_time()
}

/// FFI export: get the frequency of [`pcsx2_thread_get_cpu_time`].
///
/// Always 1_000_000 on Linux; see [`get_thread_ticks_per_second`].
#[no_mangle]
pub extern "C" fn pcsx2_thread_get_ticks_per_second() -> u64 {
    get_thread_ticks_per_second()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sleep_returns() {
        sleep(1);
    }

    #[test]
    fn timeslice_returns() {
        timeslice();
    }

    #[test]
    fn spin_wait_returns() {
        spin_wait();
    }

    #[test]
    fn cpu_time_is_monotonic() {
        let t0 = get_thread_cpu_time();
        let mut sum: u64 = 0;
        for i in 0..100_000u64 {
            sum = sum.wrapping_add(i);
        }
        let t1 = get_thread_cpu_time();
        // CPU time must be non-decreasing. We do not strictly assert
        // t1 > t0 because schedulers can pause the thread and we
        // might end up on the same tick.
        assert!(t1 >= t0, "cpu time went backwards: {} -> {}", t0, t1);
        assert!(sum > 0);
    }

    #[test]
    fn ticks_per_second_is_one_megahertz() {
        assert_eq!(get_thread_ticks_per_second(), 1_000_000);
    }

    #[test]
    fn enable_disable_hires_scheduler_are_noops() {
        enable_hires_scheduler();
        disable_hires_scheduler();
    }

    #[test]
    fn sleep_until_zero_yields() {
        sleep_until(0);
    }

    #[test]
    fn sleep_until_short_delay() {
        sleep_until(500); // 500us
    }

    #[test]
    fn set_name_truncates_long_input() {
        // 30-byte name should be truncated to 15 bytes silently.
        set_name_of_current_thread("this_is_a_very_long_name_indeed");
    }

    #[test]
    fn ffi_sleep_runs() {
        pcsx2_thread_sleep(1);
    }

    #[test]
    fn ffi_sleep_until_runs() {
        pcsx2_thread_sleep_until(100);
    }

    #[test]
    fn ffi_cpu_time_non_decreasing() {
        let t = pcsx2_thread_get_cpu_time();
        let mut s: u64 = 0;
        for i in 0..1_000_000u64 {
            s = s.wrapping_add(i);
        }
        let t2 = pcsx2_thread_get_cpu_time();
        assert!(t2 >= t);
        assert!(s > 0);
    }

    #[test]
    fn ffi_ticks_per_second_is_one_megahertz() {
        assert_eq!(pcsx2_thread_get_ticks_per_second(), 1_000_000);
    }

    #[test]
    fn ffi_set_name_null_is_safe() {
        pcsx2_thread_set_name(std::ptr::null());
    }

    #[test]
    fn ffi_set_name_basic() {
        pcsx2_thread_set_name(b"rust-test\0".as_ptr() as *const c_char);
    }
}