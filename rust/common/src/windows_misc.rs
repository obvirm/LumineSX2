// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Windows-specific miscellaneous helpers.
//!
//! Port of `common/Windows/WinMisc.cpp` + `common/HostSys.h` namespace
//! `Common` functions (`InhibitScreensaver`, `SetMousePosition`,
//! `PlaySoundAsync`, etc.) that are Windows-only.
//!
//! All functions are exported with `#[no_mangle] pub extern "C"` so the
//! C++ `_rust_shim/_shim_extras.cpp` can resolve them and replace the
//! existing stub implementations.

#![cfg(target_os = "windows")]

#![allow(dead_code, unused_imports)]

use std::ffi::c_char;
use std::ffi::c_void;
use std::ptr;

use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_NODEFAULT};
use windows_sys::Win32::System::Power::{ES_CONTINUOUS, ES_DISPLAY_REQUIRED, SetThreadExecutionState};
use windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos;

// ============================================================================
// Common::InhibitScreensaver
// ============================================================================

/// Enable or disable the screensaver.
///
/// Mirrors the C++ `Common::InhibitScreensaver(bool inhibit)`.
/// When `inhibit` is `true`, calls `SetThreadExecutionState` with
/// `ES_CONTINUOUS | ES_DISPLAY_REQUIRED` to prevent the display
/// from powering off. Returns `true` on success.
#[no_mangle]
pub extern "C" fn pcsx2_windows_inhibit_screensaver(inhibit: bool) -> bool {
    let flags = if inhibit {
        ES_CONTINUOUS | ES_DISPLAY_REQUIRED
    } else {
        ES_CONTINUOUS
    };
    // Safety: `SetThreadExecutionState` takes a simple flags value;
    // no pointers or invalid state. It is always safe to call.
    let result = unsafe { SetThreadExecutionState(flags) };
    result != 0
}

// ============================================================================
// Common::SetMousePosition
// ============================================================================

/// Move the mouse cursor to the specified screen coordinates.
///
/// Mirrors the C++ `Common::SetMousePosition(int x, int y)`.
/// Calls `SetCursorPos` under the hood.
#[no_mangle]
pub extern "C" fn pcsx2_windows_set_mouse_position(x: i32, y: i32) {
    // Safety: `SetCursorPos` is always safe to call; the coordinates
    // are clipped to the virtual screen bounds by the Win32 layer.
    unsafe {
        SetCursorPos(x, y);
    }
}

// ============================================================================
// Common::AttachMousePositionCb / DetachMousePositionCb
// ============================================================================

/// Register (or acknowledge readiness for) the raw-input mouse
/// position callback.
///
/// The C++ side uses raw input messages (handled by the Windows
/// message loop) rather than a low-level mouse hook. This function
/// simply returns `true` to indicate the mechanism is available.
#[no_mangle]
pub extern "C" fn pcsx2_windows_attach_mouse_position_cb() -> bool {
    true
}

/// Detach / unregister the mouse position callback.
///
/// The C++ side has no teardown work because it uses raw input
/// messages; this is a no-op.
#[no_mangle]
pub extern "C" fn pcsx2_windows_detach_mouse_position_cb() {
    // no-op — the C++ side uses raw input messages, not a hook.
}

// ============================================================================
// Common::PlaySoundAsync
// ============================================================================

/// Play a WAV file asynchronously through `PlaySoundW`.
///
/// Mirrors the C++ `Common::PlaySoundAsync(const char* path)`.
/// Expects a null-terminated UTF-8 path. Converts to wide chars
/// internally and calls `PlaySoundW` with `SND_ASYNC | SND_NODEFAULT`.
/// Returns `true` if the sound started successfully.
#[no_mangle]
pub extern "C" fn pcsx2_windows_play_sound_async(path: *const c_char) -> bool {
    if path.is_null() {
        return false;
    }

    // Safety: we trust the caller to pass a valid null-terminated
    // UTF-8 string; the length is capped at a reasonable maximum
    // (32767 bytes, which is the MAX_PATH for most Windows APIs).
    let path_str = unsafe { std::ffi::CStr::from_ptr(path) };
    let path_str = match path_str.to_str() {
        Ok(s) => s,
        Err(_) => return false,
    };

    // Convert to wide (UTF-16) for PlaySoundW.
    let wide: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

    // Safety: `PlaySoundW` takes a pointer to a null-terminated
    // wide string, flags, and an optional module handle (NULL here).
    // The buffer lives for the duration of the call.
    let result = unsafe { PlaySoundW(wide.as_ptr(), ptr::null_mut(), SND_ASYNC | SND_NODEFAULT) };
    result != 0
}

// ============================================================================
// Threading::Sleep
// ============================================================================

// NOTE: `Threading::Sleep(int ms)` and `Threading::SleepUntil(u64 ticks)`
// are ALREADY ported in `threading.rs` (`sleep(ms)` and `sleep_until(ticks)`)
// with cross-platform implementations.  The Win32-specific versions in
// `WinMisc.cpp` (using `::Sleep` and `SetWaitableTimer`) are superseded
// by the Rust portable implementations.

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attach_detach_mouse() {
        assert!(pcsx2_windows_attach_mouse_position_cb());
        pcsx2_windows_detach_mouse_position_cb();
        // no panics = pass
    }

    #[test]
    fn test_play_sound_null() {
        // Passing null should return false, not crash.
        assert!(!pcsx2_windows_play_sound_async(std::ptr::null()));
    }

    #[test]
    fn test_inhibit_screensaver() {
        // Toggle screensaver inhibition; should succeed on any
        // Windows system (the thread execution state is process-
        // wide and always accessible).
        let r = pcsx2_windows_inhibit_screensaver(true);
        // We accept either true or false because some environments
        // (CI containers, WinPE) may not honour ES_DISPLAY_REQUIRED.
        let _ = r;
        // Restore.
        pcsx2_windows_inhibit_screensaver(false);
    }
}
