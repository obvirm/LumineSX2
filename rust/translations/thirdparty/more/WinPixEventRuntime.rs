//! WinPixEventRuntime - idiomatic Rust 2021 translation of the
//! `WinPixEventRuntime` header.
//!
//! The Windows PIX event runtime is shipped as a DLL (see
//! `3rdparty/winpixeventruntime/bin/WinPixEventRuntime.dll`); this
//! module provides safe Rust FFI bindings and the two entry points
//! required by the spec:
//!
//! - `set_event_target(hwnd)` - bind PIX events to a window.
//! - `signal_event()` - signal a fence-style PIX event.
//!
//! The runtime functions are looked up via `LoadLibraryW` /
//! `GetProcAddress` and stored in a `static mut` cache, matching the
//! pattern used by the C++ side.

#![allow(dead_code)]
#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::OnceLock;

// Win32 FFI type for a window handle.
pub type HWND = *mut c_void;
pub type HANDLE = *mut c_void;
pub type BOOL = i32;
pub type HRESULT = i32;
pub type DWORD = u32;
pub type UINT64 = u64;

/// Library name. The real DLL is `WinPixEventRuntime.dll` in the
/// `3rdparty/winpixeventruntime/bin/` directory. On non-Windows hosts
/// this returns early without loading anything.
pub const DLL_NAME: &str = "WinPixEventRuntime.dll";

struct RuntimeFns {
    set_target_window: Option<unsafe extern "system" fn(HWND) -> HRESULT>,
    notify_wake: Option<unsafe extern "system" fn(HANDLE)>,
}

static mut RUNTIME: OnceLock<RuntimeFns> = OnceLock::new();

unsafe fn runtime() -> &'static RuntimeFns {
    if RUNTIME.get().is_none() {
        let _ = RUNTIME.set(RuntimeFns {
            set_target_window: None,
            notify_wake: None,
        });
    }
    RUNTIME.get().unwrap()
}

/// Attempt to load the WinPixEventRuntime DLL and resolve the
/// `PIXSetTargetWindow` and `PIXNotifyWakeFromFenceSignal` entry
/// points. On non-Windows hosts the function is a no-op.
pub fn load() -> bool {
    #[cfg(windows)]
    unsafe {
        use std::ffi::CString;
        let wide: Vec<u16> = DLL_NAME.encode_utf16().chain(std::iter::once(0)).collect();
        let module = LoadLibraryW(wide.as_ptr());
        if module.is_null() {
            return false;
        }
        let set_name = CString::new("PIXSetTargetWindow").unwrap();
        let notify_name = CString::new("PIXNotifyWakeFromFenceSignal").unwrap();
        let set_fn = GetProcAddress(module, set_name.as_ptr());
        let notify_fn = GetProcAddress(module, notify_name.as_ptr());
        let fns = RuntimeFns {
            set_target_window: if set_fn.is_null() {
                None
            } else {
                Some(std::mem::transmute::<*const c_void, unsafe extern "system" fn(HWND) -> HRESULT>(set_fn))
            },
            notify_wake: if notify_fn.is_null() {
                None
            } else {
                Some(std::mem::transmute::<*const c_void, unsafe extern "system" fn(HANDLE)>(notify_fn))
            },
        };
        let _ = RUNTIME.set(fns);
        true
    }
    #[cfg(not(windows))]
    {
        let _ = DLL_NAME;
        false
    }
}

/// Set the HWND that PIX events should be associated with. The C++
/// version is `PIXSetTargetWindow(HWND)`. On non-Windows hosts or if
/// the runtime DLL is not loaded, the call is a no-op.
pub fn set_event_target(hwnd: HWND) {
    unsafe {
        if let Some(f) = runtime().set_target_window {
            (f)(hwnd);
        }
    }
}

/// Signal a PIX fence event. The C++ version is
/// `PIXNotifyWakeFromFenceSignal(HANDLE)`.
pub fn signal_event() {
    unsafe {
        if let Some(f) = runtime().notify_wake {
            (f)(std::ptr::null_mut());
        }
    }
}

/// Begin/end a CPU event scope. These are no-ops in the pure-Rust
/// port - the C++ macro versions are stripped at compile time when
/// PIX is disabled.
pub fn begin_event(color: UINT64, name: &str) {
    let _ = (color, name);
}

pub fn end_event() {}

// ----- Win32 FFI (only compiled on Windows) -----
#[cfg(windows)]
extern "system" {
    fn LoadLibraryW(lp_lib_file_name: *const u16) -> *mut c_void;
    fn GetProcAddress(h_module: *mut c_void, lp_proc_name: *const i8) -> *const c_void;
    fn FreeLibrary(h_module: *mut c_void) -> i32;
}
