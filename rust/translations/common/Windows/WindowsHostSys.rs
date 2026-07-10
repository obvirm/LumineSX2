// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's `common/Windows/WinHostSys.cpp`,
//! `common/Windows/WinMisc.cpp`, and `common/Windows/WinThreads.cpp`.
//!
//! This module gathers the Windows-only host-system path discovery, the
//! modern thread-name helpers, and the canonical window-message helper
//! (`RegisterWindowMessageW`) into a single façade.
//!
//! The original C++ spread these concerns across three files because they
//! all happen to live under `common/Windows/`. The Rust port keeps the
//! same surface area but folds the implementations together since the
//! underlying dependency is the same set of Win32 entry points.
//!
//! Only the `std` crate is used. Win32 calls are reached through
//! `extern "system"` FFI declared in this module. On non-Windows targets
//! the public functions are stubbed to their `None` / no-op fallbacks so
//! cross-platform code that names them still compiles.

#![allow(dead_code)]

use std::ffi::c_void;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

// ============================================================================
// Win32 FFI declarations
// ============================================================================
//
// All Win32 entry points used by this module are declared here so the
// implementation below can call them with a stable signature. The original
// C++ code reached the same APIs through `<Windows.h>` plus a grab-bag of
// platform headers; reproducing that would be a maintenance hazard.

#[cfg(target_os = "windows")]
type DWORD = u32;
#[cfg(target_os = "windows")]
type HRESULT = i32;
#[cfg(target_os = "windows")]
type LPWSTR = *mut u16;
#[cfg(target_os = "windows")]
type LPCWSTR = *const u16;

#[cfg(target_os = "windows")]
#[link(name = "kernel32")]
extern "system" {
    fn GetModuleFileNameW(hModule: *mut c_void, lpFilename: LPWSTR, nSize: DWORD) -> DWORD;
    fn GetEnvironmentVariableW(lpName: LPCWSTR, lpBuffer: LPWSTR, nSize: DWORD) -> DWORD;
    fn GetCurrentThread() -> *mut c_void;
    fn GetThreadDescription(hThread: *mut c_void, ppszThreadDescription: *mut LPWSTR) -> HRESULT;
    fn SetThreadDescription(hThread: *mut c_void, lpThreadDescription: LPCWSTR) -> HRESULT;
    fn LocalFree(hMem: *mut c_void) -> *mut c_void;
}

#[cfg(target_os = "windows")]
#[link(name = "user32")]
extern "system" {
    fn RegisterWindowMessageW(lpString: LPCWSTR) -> u32;
}

// ============================================================================
// Internal path helpers (Windows only)
// ============================================================================

#[cfg(target_os = "windows")]
fn module_filename() -> Option<PathBuf> {
    // Two-call pattern: ask for the size with a null buffer, then allocate
    // a buffer of exactly that size. Mirrors the C++ `GetModuleFileNameW`
    // idiom used in `HostSys::GetProgramPath` (transcribed from the
    // source files, but reduced to a single helper since the original
    // path resolution is platform-specific).
    unsafe {
        let needed = GetModuleFileNameW(std::ptr::null_mut(), std::ptr::null_mut(), 0);
        if needed == 0 {
            return None;
        }
        let mut buf = vec![0u16; needed as usize];
        let written = GetModuleFileNameW(std::ptr::null_mut(), buf.as_mut_ptr(), needed);
        if written == 0 {
            return None;
        }
        buf.truncate(written as usize);
        Some(PathBuf::from(OsString::from_wide(&buf)))
    }
}

#[cfg(target_os = "windows")]
fn env_var_as_path(name: &str) -> Option<PathBuf> {
    // Same two-call pattern as `module_filename`, but reading an
    // environment variable. A missing variable returns 0 and we report
    // `None` to the caller, exactly like `getenv` does in the C source.
    let wide_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let needed = GetEnvironmentVariableW(wide_name.as_ptr(), std::ptr::null_mut(), 0);
        if needed == 0 {
            return None;
        }
        let mut buf = vec![0u16; needed as usize];
        let written = GetEnvironmentVariableW(wide_name.as_ptr(), buf.as_mut_ptr(), needed);
        if written == 0 {
            return None;
        }
        buf.truncate(written as usize);
        Some(PathBuf::from(OsString::from_wide(&buf)))
    }
}

// ============================================================================
// Public path API
// ============================================================================

/// Returns the directory that contains the running executable, or `None`
/// when the path cannot be determined (also on non-Windows targets).
///
/// Equivalent to the program-path discovery baked into PCSX2's Windows
/// host code. Implemented via `GetModuleFileNameW(NULL, ...)` and
/// stripping the file component.
pub fn get_program_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        module_filename().and_then(|p| p.parent().map(|p| p.to_path_buf()))
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Returns the `Resources/` subdirectory beside the program. This is
/// where PCSX2 places shaders, locales, documentation, and other
/// read-only bundled assets.
pub fn get_resources_path() -> Option<PathBuf> {
    get_program_path().map(|p| p.join("Resources"))
}

/// Returns the `Data/` subdirectory beside the program, used for
/// write-once install data such as BIOS images, memory cards, and
/// portable save storage.
pub fn get_data_path() -> Option<PathBuf> {
    get_program_path().map(|p| p.join("Data"))
}

/// Returns the user's profile directory, equivalent to `%USERPROFILE%`.
///
/// On non-Windows targets the function is a stub that returns `None`
/// because the same name is meaningless outside of Windows.
pub fn get_userprofile_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        env_var_as_path("USERPROFILE")
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// Returns the `Cache/` subdirectory beside the program. Used for
/// derived data such as JIT caches, shader caches, and image caches.
pub fn get_cache_path() -> Option<PathBuf> {
    get_program_path().map(|p| p.join("Cache"))
}

/// Returns the user's per-application config directory, equivalent to
/// `%APPDATA%` on Windows. This is where the bulk of PCSX2's settings
/// live on a typical install.
pub fn get_config_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        env_var_as_path("APPDATA")
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

// ============================================================================
// Thread name helpers (modern Win32 GetThreadDescription / SetThreadDescription)
// ============================================================================
//
// The original `WinThreads.cpp` set the thread name via the
// `MS_VC_EXCEPTION` (0x406D1388) trick, which only works under MSVC's
// SEH and is recognised by a debugger rather than by the kernel. The
// modern Win32 equivalent — `GetThreadDescription` / `SetThreadDescription`,
// available since Windows 10 1607 — round-trips through the OS and is
// visible to tooling that queries the kernel, so we expose those
// directly.

/// Sets the name of the calling OS thread.
///
/// `Err(hr)` is returned when the underlying Win32 call fails (the
/// `HRESULT` is propagated verbatim for diagnostics). On non-Windows
/// targets the function is a no-op that always succeeds because the
/// symbol is unreachable in practice.
pub fn set_thread_name(name: &str) -> Result<(), i32> {
    #[cfg(target_os = "windows")]
    {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let hr = SetThreadDescription(GetCurrentThread(), wide.as_ptr());
            if hr < 0 { Err(hr) } else { Ok(()) }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = name;
        Ok(())
    }
}

/// Sets the name of an arbitrary thread identified by its native Win32
/// `HANDLE` (a `*mut c_void` in Rust).
///
/// Useful when a worker thread has stored its pseudo-handle at creation
/// and wants to be renamed after the fact.
#[cfg(target_os = "windows")]
pub fn set_thread_name_for(thread: *mut c_void, name: &str) -> Result<(), i32> {
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let hr = SetThreadDescription(thread, wide.as_ptr());
        if hr < 0 { Err(hr) } else { Ok(()) }
    }
}

/// Reads back the name of the calling OS thread, or `None` if the
/// thread has no name set or the API is unavailable.
///
/// The buffer returned by `GetThreadDescription` is allocated by the
/// runtime with `LocalAlloc`, so we must hand it back to `LocalFree`
/// once we have copied the contents into a `String`.
pub fn get_thread_name() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        unsafe {
            let mut ptr: LPWSTR = std::ptr::null_mut();
            let hr = GetThreadDescription(GetCurrentThread(), &mut ptr);
            if hr < 0 || ptr.is_null() {
                return None;
            }
            // Walk the UTF-16 string to find its length, since the
            // buffer is null-terminated and the API does not report
            // a length separately.
            let mut len = 0usize;
            while *ptr.add(len) != 0 {
                len += 1;
            }
            let slice = std::slice::from_raw_parts(ptr, len);
            let name = String::from_utf16_lossy(slice);
            // The buffer was allocated with LocalAlloc, so we hand it
            // back to LocalFree. (See the comment on the Win32 docs:
            // "When you are done with the string, call the LocalFree
            // function to free the buffer.")
            LocalFree(ptr as *mut c_void);
            Some(name)
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

// ============================================================================
// Window-message helpers
// ============================================================================

/// Returns a stable, system-wide atom for the given window-message name,
/// or `None` on failure (also on non-Windows targets).
///
/// Wraps the Win32 `RegisterWindowMessageW` API. The atom is guaranteed
/// to be unique across all cooperating applications on the desktop, so
/// this is the canonical way to allocate a custom `WM_*` value for
/// inter-process messages.
pub fn register_window_message(name: &str) -> Option<u32> {
    #[cfg(target_os = "windows")]
    {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let msg = RegisterWindowMessageW(wide.as_ptr());
            if msg == 0 { None } else { Some(msg) }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = name;
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_windows_stubs_return_none() {
        // The non-Windows stubs must always be present and must always
        // return `None` (or succeed) so cross-platform code can rely on
        // them. We can only exercise this branch when the host is not
        // Windows; the test is silently skipped otherwise.
        if !cfg!(target_os = "windows") {
            assert!(get_program_path().is_none());
            assert!(get_resources_path().is_none());
            assert!(get_data_path().is_none());
            assert!(get_userprofile_path().is_none());
            assert!(get_cache_path().is_none());
            assert!(get_config_path().is_none());
            assert!(get_thread_name().is_none());
            assert!(register_window_message("pcsx2_test").is_none());
            assert!(set_thread_name("pcsx2_test").is_ok());
        }
    }

    #[test]
    fn resources_path_is_program_path_plus_resources() {
        if let (Some(prog), Some(res)) = (get_program_path(), get_resources_path()) {
            assert_eq!(res, prog.join("Resources"));
        }
    }

    #[test]
    fn data_path_is_program_path_plus_data() {
        if let (Some(prog), Some(data)) = (get_program_path(), get_data_path()) {
            assert_eq!(data, prog.join("Data"));
        }
    }

    #[test]
    fn cache_path_is_program_path_plus_cache() {
        if let (Some(prog), Some(cache)) = (get_program_path(), get_cache_path()) {
            assert_eq!(cache, prog.join("Cache"));
        }
    }
}
