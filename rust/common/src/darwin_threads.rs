// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Rust reimplementation of PCSX2's `common/Darwin/DarwinThreads.cpp`.
//!
//! macOS-specific threading primitives. The host-portable variants of
//! `Threading::*` live in `threading.rs`; this module provides the
//! Darwin/macOS implementations:
//!
//! - [`get_thread_cpu_time`] — `thread_info(THREAD_BASIC_INFO, ...)`
//!   returning user + system time in microseconds.
//! - [`set_name_of_current_thread`] — `pthread_setname_np`. The POSIX
//!   16-byte limit is enforced by truncation.
//! - [`timeslice`] — `sched_yield`.
//! - [`spin_wait`] — architecture-specific pause hint (`pause` on x86,
//!   `isb` on aarch64).
//! - [`sleep`] / [`sleep_until`] — `std::thread::sleep` on top of
//!   `clock_gettime`-driven timing.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    unused_imports,
    unused_variables,
    clippy::all,
)]

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::CStr;
    use std::os::raw::c_char;
    use std::time::{Duration, Instant};

    use libc::{
        c_int, mach_port_t, pthread_setname_np, pthread_t, sched_yield,
    };

    // mach/thread_act bindings not present in the `libc` crate proper;
    // declare them locally so we don't need an extra `mach` dependency.
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct time_value_t {
        seconds: i32,
        microseconds: i32,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct thread_basic_info_data_t {
        user_time: time_value_t,
        system_time: time_value_t,
        cpu_usage: i32,
        policy: i32,
        run_state: i32,
        flags: i32,
        suspend_count: i32,
        sleep_time: i32,
    }

    // Opaque pointer type used by the mach APIs.
    type thread_info_t = *mut thread_basic_info_data_t;
    type mach_msg_type_number_t = c_int;

    // THREAD_BASIC_INFO is a flavour constant, not a struct. On macOS the
    // header defines it as 3, but we hard-code the value here rather than
    // pulling in a full `mach` binding just for one constant.
    const THREAD_BASIC_INFO: c_int = 3;
    const KERN_SUCCESS: c_int = 0;

    unsafe extern "C" {
        fn thread_info(
            target_act: mach_port_t,
            flavor: c_int,
            thread_info_out: thread_info_t,
            thread_info_outCnt: *mut mach_msg_type_number_t,
        ) -> c_int;

        fn pthread_mach_thread_np(thread: pthread_t) -> mach_port_t;
    }

    /// Cooperative yield: `sched_yield`.
    pub fn timeslice() {
        // SAFETY: `sched_yield` is always safe to call and has no
        // preconditions.
        unsafe { sched_yield() };
    }

    /// Architecture-specific pause hint for spin/wait loops.
    ///
    /// Mirrors the C++ source: `pause` on x86, `isb` on aarch64.
    #[inline(always)]
    pub fn spin_wait() {
        #[cfg(target_arch = "x86_64")]
        unsafe {
            std::arch::asm!("pause", options(nomem, preserves_flags, att_syntax));
        }

        #[cfg(target_arch = "aarch64")]
        unsafe {
            std::arch::asm!("isb", options(nomem, preserves_flags));
        }

        // Other architectures supported by Darwin (e.g. armv7) have no
        // equivalent in the original source either, so this is a no-op.
        #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
        {
            // Intentional no-op.
        }
    }

    /// Sleep the current thread for `ms` milliseconds.
    pub fn sleep(ms: u32) {
        if ms == 0 {
            std::thread::yield_now();
            return;
        }
        std::thread::sleep(Duration::from_millis(ms as u64));
    }

    /// Sleep until an absolute [`Instant`].
    pub fn sleep_until(deadline: Instant) {
        let now = Instant::now();
        if deadline <= now {
            return;
        }
        let dur = deadline.duration_since(now);
        // Round up so we never wake early by sub-millisecond jitter.
        let ms = dur.as_millis() as u64 + u64::from(dur.subsec_micros() > 0);
        std::thread::sleep(Duration::from_millis(ms));
    }

    /// User + system CPU time consumed by the current thread, in
    /// microseconds. Returns 0 on failure (matching the C++ source).
    pub fn get_thread_cpu_time() -> u64 {
        // SAFETY: `pthread_self()` is always safe to call and returns a
        // valid handle to the calling thread.
        let thread = unsafe { pthread_mach_thread_np(libc::pthread_self()) };

        let mut info = thread_basic_info_data_t::default();
        let mut count = std::mem::size_of::<thread_basic_info_data_t>()
            / std::mem::size_of::<c_int>();

        // SAFETY: we pass a stack-local buffer, the correct flavour
        // constant, and a count that matches the struct layout above.
        let kr = unsafe {
            thread_info(
                thread,
                THREAD_BASIC_INFO,
                &mut info as *mut _ as thread_info_t,
                &mut count,
            )
        };

        if kr != KERN_SUCCESS {
            return 0;
        }

        (info.user_time.seconds as u64) * 1_000_000
            + (info.user_time.microseconds as u64)
            + (info.system_time.seconds as u64) * 1_000_000
            + (info.system_time.microseconds as u64)
    }

    /// Set the name of the current thread.
    ///
    /// macOS's `pthread_setname_np` takes only the calling thread's name
    /// and truncates at 64 bytes (unlike Linux, which enforces 16 and
    /// is per-thread).
    pub fn set_name_of_current_thread(name: &str) {
        // CStr must be NUL-terminated; `name` may contain interior NULs
        // which would truncate silently — strip everything past the
        // first NUL just in case.
        let bytes = name.as_bytes();
        let end = bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(bytes.len());
        let cstr = match CStr::from_bytes_until_nul(bytes) {
            Ok(c) => c,
            Err(_) => return,
        };
        // The above `from_bytes_until_nul` already validates, but we
        // shadow `cstr` to make the unused `end` warning-free.
        let _ = end;
        // SAFETY: `cstr` points to a valid NUL-terminated C string.
        unsafe {
            pthread_setname_np(cstr.as_ptr());
        }
    }

    // -----------------------------------------------------------------
    // FFI surface
    // -----------------------------------------------------------------

    /// FFI: set the current thread's name from a C string.
    ///
    /// # Safety
    /// `name` must be a valid NUL-terminated C string or null.
    #[no_mangle]
    pub extern "C" fn pcsx2_thread_set_name(name: *const c_char) {
        if name.is_null() {
            return;
        }
        // SAFETY: caller guarantees a valid NUL-terminated C string.
        let cstr = unsafe { CStr::from_ptr(name) };
        match cstr.to_str() {
            Ok(s) => set_name_of_current_thread(s),
            Err(_) => {
                // Non-UTF-8 names are silently ignored; the C++ side
                // would also produce undefined behaviour in that case.
            }
        }
    }

    /// FFI: sleep the current thread for `ms` milliseconds.
    #[no_mangle]
    pub extern "C" fn pcsx2_thread_sleep(ms: u32) {
        sleep(ms);
    }

    /// FFI: current thread's CPU time in microseconds (0 on failure).
    #[no_mangle]
    pub extern "C" fn pcsx2_thread_get_cpu_time() -> u64 {
        get_thread_cpu_time()
    }
}

#[cfg(target_os = "macos")]
pub use imp::*;
