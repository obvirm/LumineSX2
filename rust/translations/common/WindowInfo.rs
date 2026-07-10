// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's `common/WindowInfo.{h,cpp}`.
//!
//! The original C++ type is a platform-agnostic descriptor of an OS window
//! that is used by the graphics backends to create a swap-chain / surface
//! bound to a host window handle. It carries:
//!
//!   * the [`WindowType`] discriminant (`Win32`, `X11`, `Cocoa`,
//!     `Wayland`, `Android`, or `NoSurface`),
//!   * an opaque [`window_handle`] whose concrete meaning depends on
//!     `type_` (`HWND` on Windows, an X11 `Window` `XID` on Linux,
//!     `NSView*` on macOS, `wl_surface*` on Wayland, `ANativeWindow*`
//!     on Android, or `NULL` for `NoSurface`),
//!   * an optional [`surface_handle`] for the platforms that split the
//!     window from the drawable surface (e.g. macOS `CAMetalLayer`),
//!   * and the size in pixels used by the renderer.
//!
//! The C++ source also implements `QueryRefreshRateForWindow`, which
//! dispatches to a platform-specific helper (DWM on Windows, XRandR on
//! X11, `NSScreen.maximumRefreshRate` on macOS, ...). That logic is not
//! exposed by this module's public API -- it lives behind the
//! `get_window_info_for_rendering` entry point -- so the per-platform
//! refresh-rate probes are intentionally elided here. Callers that need
//! a refresh rate should query it through a dedicated platform module.

#![allow(dead_code)]

use std::os::raw::c_void;

/// Describes how a [`WindowInfo`] is bound to the host windowing system.
///
/// `NoSurface` is used for off-screen rendering (headless / compute-only
/// contexts) and matches the original `Type::Surfaceless` discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowType {
    /// Microsoft Windows (`HWND`).
    Win32,
    /// X11 (`Window` `XID` on a `Display*` connection).
    X11,
    /// Apple macOS (`NSView*` / `CAMetalLayer*` pair).
    Cocoa,
    /// Wayland (`wl_surface*`).
    Wayland,
    /// Android (`ANativeWindow*`).
    Android,
    /// Off-screen / headless rendering. No host window is bound.
    NoSurface,
}

/// Information required to bind a graphics context to an OS window.
///
/// Field layout and naming mirror the C++ struct so that anything which
/// already has a `WindowInfo` in hand can be ported field-for-field.
/// Handles are stored as raw `*mut c_void` pointers because their
/// concrete type depends on the [`type_`] discriminant; the caller is
/// responsible for casting them back to the appropriate platform type.
///
/// [`type_`]: WindowInfo::type_
#[derive(Debug, Clone, Copy)]
pub struct WindowInfo {
    /// The windowing system this handle is bound to.
    pub type_: WindowType,
    /// Opaque OS window handle. `HWND` on Windows, an X11 `Window` `XID`
    /// on Linux (store the value in a `usize`-sized integer cast to a
    /// pointer), `NSView*` on macOS, `wl_surface*` on Wayland,
    /// `ANativeWindow*` on Android, or `null` for [`WindowType::NoSurface`].
    pub window_handle: *mut c_void,
    /// Optional secondary surface handle. `null` on every platform
    /// except macOS, where it points to the `CAMetalLayer` paired with
    /// the `NSView` in [`window_handle`].
    pub surface_handle: *mut c_void,
    /// Surface width in pixels.
    pub width: u32,
    /// Surface height in pixels.
    pub height: u32,
}

impl Default for WindowInfo {
    /// The default value is a headless, surfaceless window of size 0x0,
    /// matching the original C++ `= Type::Surfaceless; ... = 0;` initialisers.
    fn default() -> Self {
        Self {
            type_: WindowType::NoSurface,
            window_handle: std::ptr::null_mut(),
            surface_handle: std::ptr::null_mut(),
            width: 0,
            height: 0,
        }
    }
}

// SAFETY: `WindowInfo` is a value type that owns only raw pointers. The
// pointers are not dereferenced by this module; their provenance is the
// caller's responsibility. We therefore forward the auto-traits that the
// C++ struct also satisfies (it is trivially copyable / assignable).
unsafe impl Send for WindowInfo {}
unsafe impl Sync for WindowInfo {}

/// Minimal description of a surface handed to the renderer.
///
/// The original C++ `WindowInfo` builder accepts the host's window
/// descriptor (or returns a `Surfaceless` instance when the renderer is
/// invoked headlessly). In this translation the input is represented by
/// this lightweight `Surface` struct, which already carries the window
/// handle, surface handle, and pixel dimensions the renderer needs to
/// populate a [`WindowInfo`].
#[derive(Debug, Clone, Copy)]
pub struct Surface {
    /// The windowing system this surface belongs to.
    pub type_: WindowType,
    /// OS-level window handle whose concrete type depends on `type_`.
    pub window_handle: *mut c_void,
    /// Optional secondary surface handle (e.g. `CAMetalLayer*` on macOS).
    pub surface_handle: *mut c_void,
    /// Surface width in pixels.
    pub width: u32,
    /// Surface height in pixels.
    pub height: u32,
}

impl Surface {
    /// Build a headless / surfaceless surface -- useful as a default.
    #[inline]
    pub const fn headless() -> Self {
        Self {
            type_: WindowType::NoSurface,
            window_handle: std::ptr::null_mut(),
            surface_handle: std::ptr::null_mut(),
            width: 0,
            height: 0,
        }
    }
}

/// Construct a [`WindowInfo`] suitable for handing to a renderer, given a
/// [`Surface`] descriptor.
///
/// In the original C++ this is the point at which the `WindowInfo` is
/// populated from the host's window/surface pair; we mirror that
/// behaviour with a straight field copy. The returned struct owns the
/// raw pointers it was given; the caller is responsible for keeping
/// those pointers alive for as long as the renderer uses the
/// `WindowInfo`.
///
/// This function is `unsafe` because the caller must guarantee that the
/// raw pointers in `surface` are valid for the [`WindowType`] they
/// claim to represent. The C++ constructor has the same precondition
/// but does not annotate it -- the `unsafe` keyword simply makes the
/// FFI-shaped obligation explicit on the Rust side.
#[inline]
pub unsafe fn get_window_info_for_rendering(surface: &Surface) -> WindowInfo {
    WindowInfo {
        type_: surface.type_,
        window_handle: surface.window_handle,
        surface_handle: surface.surface_handle,
        width: surface.width,
        height: surface.height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_surfaceless() {
        let wi = WindowInfo::default();
        assert_eq!(wi.type_, WindowType::NoSurface);
        assert!(wi.window_handle.is_null());
        assert!(wi.surface_handle.is_null());
        assert_eq!(wi.width, 0);
        assert_eq!(wi.height, 0);
    }

    #[test]
    fn headless_surface_round_trips() {
        let s = Surface::headless();
        // SAFETY: `s` is a fresh struct, no pointers to validate.
        let wi = unsafe { get_window_info_for_rendering(&s) };
        assert_eq!(wi.type_, WindowType::NoSurface);
        assert!(wi.window_handle.is_null());
    }

    #[test]
    fn get_window_info_for_rendering_copies_fields() {
        // A dummy, non-null pointer is fine here: the function does not
        // dereference it, it only copies the bit pattern.
        let handle = 0x1 as *mut c_void;
        let s = Surface {
            type_: WindowType::Win32,
            window_handle: handle,
            surface_handle: std::ptr::null_mut(),
            width: 1280,
            height: 720,
        };
        // SAFETY: see comment above.
        let wi = unsafe { get_window_info_for_rendering(&s) };
        assert_eq!(wi.type_, WindowType::Win32);
        assert_eq!(wi.window_handle, handle);
        assert!(wi.surface_handle.is_null());
        assert_eq!(wi.width, 1280);
        assert_eq!(wi.height, 720);
    }

    #[test]
    fn window_type_variants_are_distinct() {
        // Make sure the discriminant covers every variant the C++ header
        // mentioned (Surfaceless -> NoSurface).
        let variants = [
            WindowType::Win32,
            WindowType::X11,
            WindowType::Cocoa,
            WindowType::Wayland,
            WindowType::Android,
            WindowType::NoSurface,
        ];
        for (i, a) in variants.iter().enumerate() {
            for (j, b) in variants.iter().enumerate() {
                assert_eq!(i == j, a == b);
            }
        }
    }
}
