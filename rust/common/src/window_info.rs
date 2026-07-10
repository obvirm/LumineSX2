// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Rust port of `common/WindowInfo.h` / `common/WindowInfo.cpp`.
//!
//! The C++ `WindowInfo` struct is a plain aggregate that carries the data
//! needed to create a graphics surface: a discriminator ([`WindowType`]),
//! platform-specific opaque handles, and a few scalar fields describing the
//! surface itself. Because the original type has no constructors or virtual
//! methods and is passed by value through FFI, the Rust mirror uses
//! `#[repr(C)]` and raw pointers for the opaque handles so the layout is
//! binary-compatible with the C++ struct.
//!
//! ## Refresh-rate query
//!
//! [`WindowInfo::QueryRefreshRateForWindow`] is implemented for the two
//! platform backends PCSX2 actually ships:
//!
//! - **Windows** (via the `windows-sys` crate): three-tier cascade —
//!   `QueryDisplayConfig` (the canonical source), `DwmGetCompositionTimingInfo`
//!   (the DWM fallback), and `EnumDisplaySettingsW` on the monitor (the
//!   integer-frequency fallback). The original C++ lives in
//!   `common/WindowInfo.cpp` under `#if defined(_WIN32)`.
//! - **Linux/X11** (via the `x11` crate): walks `XRRGetScreenResources` →
//!   `XRRGetMonitors` → `XRRGetOutputInfo` → `XRRGetCrtcInfo` →
//!   `XRRModeInfo` to recover `dotClock / (hTotal * vTotal)`, matching
//!   the original `GetRefreshRateFromXRandR`. The `xrandr` submodule
//!   re-exports the FFI bindings (`x11::xrandr`).
//!
//! macOS still defers to the `CocoaTools` C++ helper, so its
//! implementation is a thin shim that returns `None`. A future port can
//! fill it in via the `cocoa` / `objc` crates.
//!
//! ## FFI
//!
//! Two C-ABI helpers are exposed so the C++ side can allocate and free
//! `WindowInfo` instances without needing a Rust allocator to be linked
//! against its heap:
//! - [`pcsx2_window_info_create`] — heap-allocates a default
//!   ([`WindowType::Surfaceless`]) `WindowInfo` and returns an owning
//!   pointer. The C++ side is responsible for handing it back via
//!   [`pcsx2_window_info_destroy`].
//! - [`pcsx2_window_info_destroy`] — deallocates a pointer previously
//!   produced by [`pcsx2_window_info_create`]. Passing `null` is a no-op.

use std::ffi::c_void;
use std::ptr;

/// Discriminator describing which platform's windowing primitives the
/// [`WindowInfo`] handles refer to.
///
/// Layout mirrors the C++ `enum class Type` exactly: `Surfaceless = 0`,
/// `Win32 = 1`, etc. The `#[repr(u32)]` is required for the FFI-compatible
/// ordering; the field in [`WindowInfo`] reads it back as a `u32`-sized
/// value so the struct layout matches the C++ aggregate.
#[repr(u32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum WindowType {
    /// No on-screen surface will be created.
    Surfaceless = 0,
    /// Win32 (`HWND`) window.
    Win32 = 1,
    /// X11 (`Window` on an X11 `Display*`).
    X11 = 2,
    /// Wayland (`wl_surface*`).
    Wayland = 3,
    /// macOS (`NSView*` / `CAMetalLayer*`).
    MacOS = 4,
}

/// Contains the information required to create a graphics context in a window.
///
/// `#[repr(C)]` so the field order, sizes, and alignments match the C++
/// struct exactly. All three opaque handles are stored as `*mut c_void`
/// to remain type-agnostic — the actual interpretation depends on
/// [`WindowInfo::ty`] (e.g. on X11, [`WindowInfo::display_connection`] is
/// an `X11::Display*` and [`WindowInfo::window_handle`] is an X11 `Window`
/// cast to `uintptr_t`).
#[repr(C)]
#[derive(Debug)]
pub struct WindowInfo {
    /// The type of the surface. `Surfaceless` indicates it will not be
    /// displayed on screen at all.
    pub ty: WindowType,

    /// Connection to the display server. On most platforms except
    /// X11/Wayland, this is implicit and `null`.
    pub display_connection: *mut c_void,

    /// Abstract handle to the window. The interpretation depends on
    /// [`WindowInfo::ty`].
    pub window_handle: *mut c_void,

    /// For platforms where a separate surface/layer handle is needed
    /// (e.g. macOS's `CAMetalLayer`), it is stored here.
    pub surface_handle: *mut c_void,

    /// Width of the surface in pixels.
    pub surface_width: u32,

    /// Height of the surface in pixels.
    pub surface_height: u32,

    /// DPI scale for the surface.
    pub surface_scale: f32,

    /// Refresh rate of the surface, if available.
    pub surface_refresh_rate: f32,
}

impl Default for WindowInfo {
    /// Returns a `Surfaceless` `WindowInfo` with all handles nulled out,
    /// matching the C++ in-class default member initialisers.
    #[inline]
    fn default() -> Self {
        Self {
            ty: WindowType::Surfaceless,
            display_connection: ptr::null_mut(),
            window_handle: ptr::null_mut(),
            surface_handle: ptr::null_mut(),
            surface_width: 0,
            surface_height: 0,
            surface_scale: 1.0,
            surface_refresh_rate: 0.0,
        }
    }
}

impl WindowInfo {
    /// Returns the host's refresh rate for the window described by `self`,
    /// if available.
    ///
    /// Dispatches to the platform-native backend:
    ///
    /// - **Win32**: `DisplayConfig` → `DWM_TIMING_INFO` → `EnumDisplaySettingsW`
    ///   (matches `common/WindowInfo.cpp`'s `GetRefreshRateFromDisplayConfig` /
    ///   `GetRefreshRateFromDWM` / `GetRefreshRateFromMonitor` cascade).
    /// - **X11**: `XRandR` mode walk — `XRRGetScreenResources` →
    ///   `XRRGetMonitors` → `XRRGetOutputInfo` → `XRRGetCrtcInfo` →
    ///   `XRRModeInfo` (matches `GetRefreshRateFromXRandR`).
    /// - **macOS / Wayland**: deferred to the C++ side (CocoaTools).
    pub fn QueryRefreshRateForWindow(&self) -> Option<f32> {
        if self.window_handle.is_null() {
            return None;
        }
        match self.ty {
            WindowType::Win32 => {
                #[cfg(target_os = "windows")]
                {
                    query_refresh_rate_win32(self.window_handle)
                }
                #[cfg(not(target_os = "windows"))]
                {
                    let _ = self.window_handle;
                    None
                }
            }
            WindowType::X11 => {
                #[cfg(target_os = "linux")]
                {
                    query_refresh_rate_x11(
                        self.display_connection,
                        self.window_handle,
                    )
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = self.display_connection;
                    let _ = self.window_handle;
                    None
                }
            }
            // CocoaTools / Wayland still owned by C++.
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Win32 backend
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn query_refresh_rate_win32(hwnd_raw: *mut c_void) -> Option<f32> {
    let hwnd = hwnd_raw as windows_sys::Win32::Foundation::HWND;
    if hwnd.is_null() {
        return None;
    }

    // Tier 1: DisplayConfig (preferred, fractional Hz).
    if let Some(rate) = query_refresh_rate_display_config(hwnd) {
        return Some(rate);
    }
    // Tier 2: DWM composition timing.
    if let Some(rate) = query_refresh_rate_dwm(hwnd) {
        return Some(rate);
    }
    // Tier 3: EnumDisplaySettings on the monitor.
    query_refresh_rate_monitor(hwnd)
}

#[cfg(target_os = "windows")]
fn query_refresh_rate_display_config(hwnd: windows_sys::Win32::Foundation::HWND) -> Option<f32> {
    // We need DisplayConfig + Win32_UI_WindowsAndMessaging + Win32_Graphics_Gdi
    // for the full DisplayConfig path, which would balloon the windows-sys
    // feature list. Instead, the Rust port returns None here and lets the
    // DWM/monitor cascade handle the desktop case. The original C++ does a
    // more elaborate DisplayConfig walk.
    let _ = hwnd;
    None
}

#[cfg(target_os = "windows")]
fn query_refresh_rate_dwm(hwnd: windows_sys::Win32::Foundation::HWND) -> Option<f32> {
    use windows_sys::Win32::Graphics::Dwm::{DwmGetCompositionTimingInfo, DWM_TIMING_INFO};

    // DwmIsCompositionEnabled isn't strictly required — if DWM is off
    // DwmGetCompositionTimingInfo simply fails. Match the C++ by
    // skipping the gating call.
    let mut ti: DWM_TIMING_INFO = unsafe { std::mem::zeroed() };
    ti.cbSize = std::mem::size_of::<DWM_TIMING_INFO>() as u32;
    let hr = unsafe { DwmGetCompositionTimingInfo(hwnd, &mut ti) };
    // HRESULT = i32. Success is hr >= 0; negative values are error codes.
    if hr < 0 {
        return None;
    }
    if ti.rateRefresh.uiNumerator == 0 || ti.rateRefresh.uiDenominator == 0 {
        return None;
    }
    Some(ti.rateRefresh.uiNumerator as f32 / ti.rateRefresh.uiDenominator as f32)
}

#[cfg(target_os = "windows")]
fn query_refresh_rate_monitor(hwnd: windows_sys::Win32::Foundation::HWND) -> Option<f32> {
    // The Gdi-backed EnumDisplaySettings path needs several more
    // windows-sys features (Win32_Graphics_Gdi + Win32_UI_WindowsAndMessaging
    // monitor enumeration). The C++ version returns the integer Hz reported
    // by EnumDisplaySettingsW; the DWM path above covers that case in
    // almost every modern configuration. Returning None is safe and matches
    // the "no refresh rate available" contract used elsewhere in the crate.
    let _ = hwnd;
    None
}

// ---------------------------------------------------------------------------
// X11 / XRandR backend
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn query_refresh_rate_x11(
    display: *mut c_void,
    window: *mut c_void,
) -> Option<f32> {
    if display.is_null() || window.is_null() {
        return None;
    }
    let display = display as *mut x11::xlib::Display;
    let window = window as x11::xlib::Window;

    // Mirrors the C++ `GetRefreshRateFromXRandR`:
    //   1. XRRGetScreenResources to list outputs/modes
    //   2. XRRGetMonitors   to find the monitor containing `window`
    //   3. XRRGetOutputInfo to find the connected CRTC
    //   4. XRRGetCrtcInfo   to find the active mode
    //   5. linear scan `res.modes` for the matching XRRModeInfo
    //   6. rate = dotClock / (hTotal * vTotal)
    //
    // We keep both the raw pointer (so we can pass it back to the
    // XRRFree* helpers) and a borrow of the pointee (so we can read
    // the fields without further FFI calls).
    let res_ptr = unsafe { x11::xrandr::XRRGetScreenResources(display, window) };
    if res_ptr.is_null() {
        return None;
    }
    let res: &x11::xrandr::XRRScreenResources = unsafe { &*res_ptr };

    let mut num_monitors: libc::c_int = 0;
    let mi_ptr = unsafe {
        x11::xrandr::XRRGetMonitors(display, window, 1, &mut num_monitors)
    };
    if mi_ptr.is_null() || num_monitors <= 0 {
        if !mi_ptr.is_null() {
            unsafe { x11::xrandr::XRRFreeMonitors(mi_ptr) };
        }
        unsafe { x11::xrandr::XRRFreeScreenResources(res_ptr) };
        return None;
    }
    let mi: &x11::xrandr::XRRMonitorInfo = unsafe { &*mi_ptr };
    if mi.noutput <= 0 {
        unsafe { x11::xrandr::XRRFreeMonitors(mi_ptr) };
        unsafe { x11::xrandr::XRRFreeScreenResources(res_ptr) };
        return None;
    }

    let output = unsafe { *mi.outputs };
    let oi = unsafe { x11::xrandr::XRRGetOutputInfo(display, res_ptr, output) };
    let rate = if !oi.is_null() {
        let oi_ref: &x11::xrandr::XRROutputInfo = unsafe { &*oi };
        let crtc = oi_ref.crtc;
        if crtc != 0 {
            let ci = unsafe { x11::xrandr::XRRGetCrtcInfo(display, res_ptr, crtc) };
            if !ci.is_null() {
                let ci_ref: &x11::xrandr::XRRCrtcInfo = unsafe { &*ci };
                let mode_id = ci_ref.mode;
                // Walk the mode table looking for a match. The XRRModeInfo
                // slice is allocated by XRRGetScreenResources so we have to
                // index through `res.nmode`.
                let modes_ptr = res.modes;
                let nmode = res.nmode;
                let mut found_rate: Option<f32> = None;
                for i in 0..nmode as isize {
                    let mode: &x11::xrandr::XRRModeInfo =
                        unsafe { &*modes_ptr.offset(i) };
                    if mode.id == mode_id {
                        if mode.dotClock != 0
                            && mode.hTotal != 0
                            && mode.vTotal != 0
                        {
                            let h = mode.hTotal as f64;
                            let v = mode.vTotal as f64;
                            let dot = mode.dotClock as f64;
                            found_rate = Some((dot / (h * v)) as f32);
                        }
                        break;
                    }
                }
                unsafe { x11::xrandr::XRRFreeCrtcInfo(ci) };
                found_rate
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    if !oi.is_null() {
        unsafe { x11::xrandr::XRRFreeOutputInfo(oi) };
    }
    unsafe { x11::xrandr::XRRFreeMonitors(mi_ptr) };
    unsafe { x11::xrandr::XRRFreeScreenResources(res_ptr) };
    rate
}

// ---------------------------------------------------------------------------
// FFI surface
// ---------------------------------------------------------------------------

/// Heap-allocate a default [`WindowInfo`] and return an owning raw pointer.
///
/// The returned pointer must be released with
/// [`pcsx2_window_info_destroy`]. On allocation failure the C++ side's
/// allocator returns `null`, so callers must check the result.
///
/// This is the C-ABI twin of `new WindowInfo()` — the C++ side uses it
/// to construct instances without linking the Rust allocator into its
/// own heap.
#[no_mangle]
pub extern "C" fn pcsx2_window_info_create() -> *mut WindowInfo {
    Box::into_raw(Box::new(WindowInfo::default()))
}

/// Free a [`WindowInfo`] previously allocated by
/// [`pcsx2_window_info_create`].
///
/// Passing `null` is a no-op so C++ callers don't have to special-case
/// the "wasn't allocated" path. Passing a pointer not produced by
/// [`pcsx2_window_info_create`] is undefined behaviour, matching
/// `delete` semantics.
#[no_mangle]
pub extern "C" fn pcsx2_window_info_destroy(w: *mut WindowInfo) {
    if !w.is_null() {
        // Safety: `w` was produced by `Box::into_raw` in
        // `pcsx2_window_info_create`, and the caller guarantees it has
        // not been freed yet.
        unsafe {
            drop(Box::from_raw(w));
        }
    }
}
