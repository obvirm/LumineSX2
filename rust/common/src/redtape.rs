// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Windows COM wrappers and helpers.
//!
//! Rust port of `common/RedtapeWilCom.h` and `common/RedtapeWindows.h`.
//!
//! In the C++ codebase these headers:
//!
//! - `RedtapeWindows.h` — includes `<Windows.h>` with `WIN32_LEAN_AND_MEAN`
//!   and `NOMINMAX`, and sets `_WIN32_WINNT` to `0x0A00` (Windows 10).
//!   In Rust these defines are irrelevant (the `windows` / `windows-sys`
//!   crates handle platform targeting via Cargo features).
//!
//! - `RedtapeWilCom.h` — pulls in `<wil/com.h>` for `wil::com_ptr<T>`,
//!   `wil::unique_couninitialize_call`, etc.  In Rust the equivalent
//!   functionality is provided by the `windows` crate's COM interface
//!   support plus simple RAII guards.
//!
//! # What this module provides
//!
//! | C++ pattern                | Rust equivalent                                     |
//! |----------------------------|-----------------------------------------------------|
//! | `wil::com_ptr<T>`          | `windows::core::ComPtr<T>`                          |
//! | `wil::unique_couninitialize_call` | [`ComInitScope`]                               |
//! | `CoCreateInstance`         | `windows::core::ComObject::new()` / direct FFI      |
//! | `WIN32_LEAN_AND_MEAN`      | Not needed in Rust                                  |
//! | `_WIN32_WINNT = 0x0A00`    | Not needed in Rust                                  |
//!
//! # Usage
//!
//! ```rust,ignore
//! use pcsx2_common_rs::redtape::ComInitScope;
//!
//! // On the thread that needs COM (e.g. boot thread):
//! let _com = ComInitScope::new();
//! // COM calls are now available; `_com` is dropped on scope exit,
//! // which calls `CoUninitialize`.
//! ```

#![cfg(target_os = "windows")]

#![allow(dead_code)]

use std::ffi::c_void;
use std::ptr;

use windows_sys::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

// ============================================================================
// ComInitScope — RAII guard for COM apartment initialisation
// ============================================================================

/// RAII guard that calls `CoInitializeEx(COINIT_MULTITHREADED)` on
/// construction and `CoUninitialize` on drop.
///
/// Equivalent to `wil::unique_couninitialize_call` in the WIL library.
/// Use this at the start of any thread that needs to make COM calls
/// (e.g. the PCSX2 boot thread, DSound audio thread, etc.).
///
/// # Panics
///
/// Does **not** panic if COM was already initialised on this thread
/// (S_FALSE is treated as success).  Panics only in debug builds if
/// the initialisation fails with an unexpected HRESULT.
///
/// # Example
///
/// ```rust,ignore
/// {
///     let _com = ComInitScope::new();
///     // COM calls are valid here...
/// }
/// // CoUninitialize called automatically.
/// ```
pub struct ComInitScope {
    _private: (),
}

impl ComInitScope {
    /// Initialise COM on the calling thread with
    /// `COINIT_MULTITHREADED`.
    ///
    /// If COM was already initialised on this thread (e.g. by the
    /// Slint/winit event loop) the call returns S_FALSE, which is
    /// treated as success — the corresponding `CoUninitialize`
    /// will still be called on drop, matching WIL's behaviour.
    ///
    /// # Panics
    ///
    /// In debug builds, panics if `CoInitializeEx` returns any
    /// value other than S_OK (0) or S_FALSE (1).
    pub fn new() -> Self {
        // Safety: `CoInitializeEx` is called with a null reserved
        // pointer, which is the documented requirement.
        let hr = unsafe { CoInitializeEx(ptr::null(), COINIT_MULTITHREADED as u32) };
        debug_assert!(
            hr == 0 || hr == 1,
            "CoInitializeEx(COINIT_MULTITHREADED) failed with HRESULT {:#x}",
            hr,
        );
        Self { _private: () }
    }

    /// Initialise COM with a custom concurrency model.
    ///
    /// Normally `new()` (which uses `COINIT_MULTITHREADED`) is
    /// preferred. Use this variant only when the calling thread
    /// explicitly requires `COINIT_APARTMENTTHREADED`.
    ///
    /// # Safety
    ///
    /// The caller must ensure the concurrency model is compatible
    /// with the COM objects used on this thread.
    pub unsafe fn with_model(concurrency_model: u32) -> Self {
        let hr = CoInitializeEx(ptr::null(), concurrency_model);
        debug_assert!(
            hr == 0 || hr == 1,
            "CoInitializeEx({:#x}) failed with HRESULT {:#x}",
            concurrency_model,
            hr,
        );
        Self { _private: () }
    }
}

impl Drop for ComInitScope {
    fn drop(&mut self) {
        // Safety: `CoUninitialize` has no requirements beyond
        // having previously called `CoInitializeEx` on the same
        // thread, which the constructor guarantees.
        unsafe {
            CoUninitialize();
        }
    }
}

// ============================================================================
// CoTaskMemFree helper
// ============================================================================

/// Free memory allocated by a COM API via `CoTaskMemFree`.
///
/// Wraps the Win32 `CoTaskMemFree` call.  Use with pointers
/// returned by COM methods that document "caller must free with
/// `CoTaskMemFree`".
///
/// # Safety
///
/// `ptr` must have been allocated by a COM API that uses the
/// task allocator.  Passing any other pointer is undefined
/// behaviour.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_redtape_co_task_mem_free(ptr: *mut c_void) {
    if !ptr.is_null() {
        // Safety: delegated to caller
        windows_sys::Win32::System::Com::CoTaskMemFree(ptr);
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_com_init_scope() {
        // Basic RAII: initialise and uninitialise COM on a fresh
        // test thread.  The test runner may already have COM
        // initialised, so we spin up a dedicated thread.
        let handle = std::thread::spawn(|| {
            let _scope = ComInitScope::new();
            // Inside the scope COM is available.
            // (No specific call tested — the important thing is
            // that no panic occurs.)
        });
        handle.join().expect("ComInitScope thread panicked");
    }

    #[test]
    fn test_co_task_mem_free_null() {
        // Passing null should be a no-op.
        unsafe {
            pcsx2_redtape_co_task_mem_free(std::ptr::null_mut());
        }
        // no crash = pass
    }
}
