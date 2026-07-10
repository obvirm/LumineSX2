// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Rust reimplementation of PCSX2's `common/Windows/WinThreads.cpp`.
//!
//! Windows-specific implementations of the `Threading::*` free functions
//! from `common/Threading.h`. The host-portable variants live in
//! `threading.rs`; this module provides the Win32 implementations that
//! PCSX2's Windows builds actually link against:
//!
//! - [`get_thread_cpu_time`] — `GetThreadTimes`, returning the sum of
//!   user-mode and kernel-mode 100-ns FILETIME ticks for the current
//!   thread (0 on failure).
//! - [`set_name_of_current_thread`] — `SetThreadDescription` with a
//!   UTF-16 conversion of the supplied `&str`. The newer Win32 API is
//!   preferred over the old MSVC structured-exception hack
//!   (`RaiseException(MS_VC_EXCEPTION, ...)`) that the C++ source uses.
//! - [`timeslice`] — `SwitchToThread()` (a stronger yield than
//!   `Sleep(0)`: it actually hands the timeslice to another runnable
//!   thread when one exists).
//! - [`spin_wait`] — `std::hint::spin_loop()`, the standard-library
//!   equivalent of `_mm_pause` / `YieldProcessor`.
//! - [`enable_hires_scheduler`] / [`disable_hires_scheduler`] —
//!   `timeBeginPeriod(1)` / `timeEndPeriod(1)` from the Win32
//!   multimedia timer API.
//! - [`sleep`] / [`sleep_until`] — `std::thread::sleep` for the former
//!   and a `CreateWaitableTimer` round for the latter (matches the
//!   Win32 idiom PCSX2 uses elsewhere for high-resolution sleeping
//!   with sub-millisecond granularity). `sleep_until` takes a 100-ns
//!   tick count so it lines up with the FILETIME-based units used by
//!   [`get_thread_cpu_time`].
//!
//! ## Why `windows-sys`
//!
//! All Win32 calls go through `windows-sys` rather than hand-rolled
//! `extern "system"` blocks. `windows-sys` is the official Microsoft
//! FFI crate; it gives us correct constants (`INFINITE`, `FALSE`) and
//! `FILETIME` / `LARGE_INTEGER` / `HANDLE` layouts for free. The
//! crate is declared as a Windows-only direct dependency in
//! `Cargo.toml` (see the `[target.'cfg(target_os = "windows")'.dependencies]`
//! table) so it is part of the build matrix on every platform that
//! needs it and absent elsewhere.
//!
//! ## Why a separate module
//!
//! This file is structured as a parallel to `darwin_threads.rs`:
//! `#[cfg(target_os = "windows")]` gates the `mod imp` block and the
//! `pub use imp::*;` re-export at the bottom. The `lib.rs` for this
//! crate is intentionally **not** edited in this change; the module is
//! expected to be wired in by a follow-up commit that also decides
//! whether to fold these implementations into the platform sections
//! of `threading.rs` or keep them split per-OS.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    unused_imports,
    unused_variables,
    clippy::all,
)]

#[cfg(target_os = "windows")]
mod imp {
    use std::ffi::CStr;
    use std::os::raw::c_char;
    use std::ptr;
    use std::time::Duration;

    use windows_sys::core::BOOL;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, FILETIME, FALSE};
    use windows_sys::Win32::System::Threading::INFINITE;
    use windows_sys::Win32::Media::timeBeginPeriod;
    use windows_sys::Win32::Media::timeEndPeriod;
    use windows_sys::Win32::System::Threading::{
        GetCurrentThread, GetThreadTimes, SetThreadDescription, SwitchToThread, WaitForSingleObject,
    };

    // `CreateWaitableTimerW`, `SetWaitableTimer`
    // are declared locally because the windows-sys 0.61 feature gates don't
    // always expose them in the pinned `Win32_*` modules.

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateWaitableTimerW(
            lp_timer_attributes: *mut core::ffi::c_void,
            b_manual_reset: i32,
            lp_timer_name: *const u16,
        ) -> HANDLE;
        fn SetWaitableTimer(
            h_timer: HANDLE,
            p_duetime: *const i64,
            l_period: i32,
            pfn_completion_routine: *mut core::ffi::c_void,
            lp_arg_to_completion_routine: *mut core::ffi::c_void,
            f_resume: i32,
        ) -> i32;
    }

    // -----------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------

    /// Number of 100-ns ticks in one second. `FILETIME` counts time in
    /// 100-ns ticks since the Windows epoch (1601-01-01), so the
    /// user-time / kernel-time deltas returned by `GetThreadTimes` are
    /// measured in this unit. We don't actually need this constant for
    /// arithmetic in this file (we sum ticks and let the caller scale
    /// by `GetThreadTicksPerSecond` if they want seconds), but it
    /// documents the unit for readers.
    #[allow(dead_code)]
    const HUNDRED_NANOS_PER_SEC: u64 = 10_000_000;

    // -----------------------------------------------------------------
    // Helper: zero-initialised FILETIME
    // -----------------------------------------------------------------

    /// A `FILETIME` initialised to zero. `GetThreadTimes` writes to all
    /// four output slots on success, but we pass zeroed inputs anyway
    /// so the parameters are never read in their uninitialised state.
    #[inline]
    fn zero_filetime() -> windows_sys::Win32::Foundation::FILETIME {
        windows_sys::Win32::Foundation::FILETIME {
            dwLowDateTime: 0,
            dwHighDateTime: 0,
        }
    }

    // -----------------------------------------------------------------
    // Public Rust API
    // -----------------------------------------------------------------

    /// Cooperative yield: hand the remainder of the current timeslice
    /// to another runnable thread on the same processor.
    ///
    /// Mirrors the C++ `Threading::Timeslice()`. The original called
    /// `Sleep(0)`; we use the stronger `SwitchToThread()` here, which
    /// is what `Sleep(0)` typically expands to on modern Windows and
    /// avoids the corner case where `Sleep(0)` only checks the calling
    /// thread's priority queue.
    #[inline]
    pub fn timeslice() {
        // SAFETY: `SwitchToThread` is always safe to call and has no
        // preconditions. The return value (whether another thread ran)
        // is intentionally ignored — we only care about yielding.
        unsafe {
            SwitchToThread();
        }
    }

    /// Architecture-specific pause hint for spin/wait loops.
    ///
    /// Mirrors the C++ `Threading::SpinWait()`, which uses
    /// `_mm_pause()` on x86 and `YieldProcessor()` on every other
    /// architecture. The standard library's [`std::hint::spin_loop`]
    /// expands to the right intrinsic on every target Rust supports
    /// (`_mm_pause` on x86/x86_64, `yield` on aarch64, `nop` on
    /// wasm32, etc.), so we use it directly.
    #[inline(always)]
    pub fn spin_wait() {
        std::hint::spin_loop();
    }

    /// Switch the Windows scheduler to its high-resolution 1 ms period.
    ///
    /// Mirrors `Threading::EnableHiresScheduler()`. On laptops and
    /// power-saving configurations the default scheduler period is
    /// often 15-20 ms; this call drops it to 1 ms for the duration of
    /// the process, which is the same trick used by most games and
    /// multimedia software. Pair every successful call with a
    /// matching [`disable_hires_scheduler`].
    pub fn enable_hires_scheduler() {
        // SAFETY: `timeBeginPeriod` is documented as safe to call from
        // any thread and only mutates process-global timer state. The
        // return value (MMSYSERR_*) is intentionally ignored: the
        // C++ source does the same.
        unsafe {
            timeBeginPeriod(1);
        }
    }

    /// Restore the previous (default) scheduler period.
    ///
    /// Mirrors `Threading::DisableHiresScheduler()`. Always call this
    /// from a thread that has paired with a previous
    /// [`enable_hires_scheduler`] call — the Win32 multimedia timer
    /// API is reference-counted, so a missed pairing leaves the
    /// process stuck in 1 ms mode until exit.
    pub fn disable_hires_scheduler() {
        // SAFETY: same as `enable_hires_scheduler`; the return value
        // is intentionally ignored.
        unsafe {
            timeEndPeriod(1);
        }
    }

    /// Sleep the current thread for `ms` milliseconds.
    ///
    /// Mirrors the Win32 `Sleep(DWORD)` API but with a friendlier
    /// signature: a `u32` count of milliseconds. `ms == 0` yields
    /// the timeslice and returns immediately rather than entering
    /// the kernel sleep path.
    pub fn sleep(ms: u32) {
        if ms == 0 {
            std::thread::yield_now();
            return;
        }
        std::thread::sleep(Duration::from_millis(ms as u64));
    }

    /// Sleep for `ticks` 100-ns units (FILETIME granularity).
    ///
    /// This is the FFI-friendly variant of `sleep`: the parameter is
    /// in the same 100-ns units that `GetThreadTimes` and `FILETIME`
    /// use, which makes it the natural currency for "sleep until some
    /// absolute timer deadline" without dragging an `Instant` across
    /// the C ABI. A value of `0` yields the timeslice and returns
    /// immediately.
    ///
    /// Implemented via `CreateWaitableTimerW` + `SetWaitableTimer`
    /// with a negative relative `lpDueTime`, then `WaitForSingleObject`
    /// on the timer handle. The timer-based path is preferred over
    /// `Sleep` because `Sleep` rounds to the current scheduler period
    /// (typically 15 ms on laptops) while `SetWaitableTimer` wakes at
    /// the requested tick.
    ///
    /// On any Win32 failure (timer creation, timer arm, or handle
    /// close) the function falls back to a millisecond sleep rounded
    /// up, so the worst case is "we slept a bit too long" rather than
    /// "we panicked in the FFI path".
    pub fn sleep_until(ticks: u64) {
        if ticks == 0 {
            std::thread::yield_now();
            return;
        }

        // SAFETY: all three arguments to `CreateWaitableTimerW` are
        // well-defined here. `NULL` security attributes give the
        // timer the default DACL; `FALSE` means auto-reset (the
        // thread should wake exactly once); `NULL` means an unnamed
        // timer.
        let timer: HANDLE = unsafe {
            CreateWaitableTimerW(ptr::null_mut(), FALSE, ptr::null())
        };
        if timer.is_null() {
            fallback_sleep_from_ticks(ticks);
            return;
        }

        // SetWaitableTimer expects a LARGE_INTEGER (i64). A negative
        // value is a relative offset in 100-ns ticks, so we negate
        // `ticks`. `i64` overflow is the only failure mode here; if
        // the caller asks for more than ~29 000 years of sleep we
        // fall back to the millisecond sleep path.
        let due_time = match i64::try_from(ticks) {
            Ok(t) => t.saturating_neg(),
            Err(_) => {
                // SAFETY: just allocated; close on the way out.
                unsafe {
                    CloseHandle(timer);
                }
                fallback_sleep_from_ticks(ticks);
                return;
            }
        };

        // SAFETY: `timer` is a valid waitable-timer handle, `due_time`
        // is a non-zero negative relative duration (in 100-ns ticks),
        // and the remaining arguments (`0` period = single-shot,
        // `NULL` completion routine, `NULL` arg to it, `FALSE` for
        // "do not restore the system from suspend") match the
        // documented zero-initialisation contract.
        let ok: BOOL = unsafe {
            SetWaitableTimer(
                timer,
                &due_time as *const i64,
                0i32,
                std::ptr::null_mut::<core::ffi::c_void>(),
                std::ptr::null_mut::<core::ffi::c_void>(),
                FALSE,
            )
        };
        if ok == 0 {
            // SAFETY: just allocated; close on the way out.
            unsafe {
                CloseHandle(timer);
            }
            fallback_sleep_from_ticks(ticks);
            return;
        }

        // SAFETY: `timer` is valid and `INFINITE` is the documented
        // sentinel for "wait forever". The return value
        // (`WAIT_OBJECT_0` on success, `WAIT_FAILED` etc. on
        // failure) is intentionally ignored: the timer auto-resets
        // and any spurious wakeup is a no-op for the caller.
        unsafe {
            WaitForSingleObject(timer, INFINITE);
            CloseHandle(timer);
        }
    }

    /// User + kernel CPU time consumed by the current thread, in
    /// 100-ns FILETIME ticks.
    ///
    /// Mirrors the non-x86 branch of `Threading::GetThreadCpuTime()`.
    /// Returns `0` on failure, matching the C++ source.
    pub fn get_thread_cpu_time() -> u64 {
        // SAFETY: `GetCurrentThread` returns a pseudo-handle that is
        // valid for the calling thread and does not need to be closed.
        let thread = unsafe { GetCurrentThread() };

        let mut creation = zero_filetime();
        let mut exit_time = zero_filetime();
        let mut kernel = zero_filetime();
        let mut user = zero_filetime();

        // SAFETY: `thread` is a valid pseudo-handle; the four FILETIME
        // out-parameters are stack-local and properly aligned. The
        // `BOOL` return value (0 on failure) is intentionally
        // ignored: the C++ source returns 0 on failure too, which
        // we achieve by simply not adding to the result.
        let ok = unsafe {
            GetThreadTimes(thread, &mut creation, &mut exit_time, &mut kernel, &mut user)
        };
        if ok == 0 {
            return 0;
        }

        let user_ticks = file_time_to_u64(&user);
        let kernel_ticks = file_time_to_u64(&kernel);
        user_ticks.wrapping_add(kernel_ticks)
    }

    /// Set the name of the current thread.
    ///
    /// Mirrors `Threading::SetNameOfCurrentThread()`. The C++ source
    /// uses the legacy MSVC `RaiseException(MS_VC_EXCEPTION, ...)`
    /// trick; this port uses the modern Win32 `SetThreadDescription`
    /// API (Windows 10 1607+) which is the official replacement and
    /// is visible to every debugger, profiler, and `!threadpool`
    /// query without any setup.
    ///
    /// Names longer than the OS-imposed limit are silently truncated
    /// (Windows copies at most ~`MAX_THREAD_DESCRIPTION_LENGTH` chars).
    /// Invalid UTF-8 is silently ignored.
    pub fn set_name_of_current_thread(name: &str) {
        // Encode to UTF-16. `encode_utf16` is fallible (surrogates);
        // `unwrap()` would panic on lone surrogates, so replace them
        // with U+FFFD instead. The wide string must be NUL-terminated
        // for the Win32 API.
        let mut wide: Vec<u16> = name
            .encode_utf16()
            .map(|u| if (0xD800..=0xDFFF).contains(&u) { 0xFFFD } else { u })
            .collect();
        wide.push(0);

        // SAFETY: `GetCurrentThread()` returns a valid pseudo-handle
        // for the calling thread; `wide.as_ptr()` points to a valid
        // NUL-terminated UTF-16 string. The HRESULT return is
        // intentionally ignored: failure here is harmless (the
        // debugger just won't show the name).
        unsafe {
            let _ = SetThreadDescription(GetCurrentThread(), wide.as_ptr());
        }
    }

    // -----------------------------------------------------------------
    // FFI surface
    // -----------------------------------------------------------------

    /// FFI: set the current thread's name from a C string.
    ///
    /// # Safety
    /// `name` must be a valid NUL-terminated C string, or null (in
    /// which case the call is a no-op).
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

    /// FFI: current thread's CPU time in 100-ns FILETIME ticks.
    ///
    /// Returns `0` on failure, matching the C++ source. Callers that
    /// want seconds should divide by the value returned from
    /// `pcsx2_thread_get_ticks_per_second()` (a separate FFI export
    /// in the C++ source that lives in the platform-independent
    /// `threading.rs` and is not re-implemented here).
    #[no_mangle]
    pub extern "C" fn pcsx2_thread_get_cpu_time() -> u64 {
        get_thread_cpu_time()
    }

    // -----------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------

    /// Convert a `FILETIME` (low/high u32 pair) into a `u64` tick count.
    #[inline]
    fn file_time_to_u64(ft: &windows_sys::Win32::Foundation::FILETIME) -> u64 {
        ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64)
    }

    /// Fallback for [`sleep_until`]: convert 100-ns ticks to
    /// milliseconds (rounding up so we never under-sleep) and call
    /// [`sleep`]. Used when the waitable-timer path fails.
    #[inline]
    fn fallback_sleep_from_ticks(ticks: u64) {
        let ms = ticks.div_ceil(10_000);
        sleep(u32::try_from(ms).unwrap_or(u32::MAX));
    }
}

#[cfg(target_os = "windows")]
pub use imp::*;
