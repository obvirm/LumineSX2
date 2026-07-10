// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's `common/CocoaTools.h`.
//!
//! The original C/C++ header is gated on `__APPLE__` and centralises the
//! handful of helpers that need to call into Objective-C / Cocoa to
//! manipulate the host application: managing the `CAMetalLayer` attached
//! to an `NSWindow`, locating the application bundle (including the
//! non-translocated path), moving files to the trash, launching other
//! applications, opening Finder, and driving the Cocoa run loop.
//!
//! Rust cannot speak Objective-C directly, so on macOS this module is a
//! collection of `extern "C"` shims that forward to the existing C++
//! implementation in `common/CocoaTools.mm` (or, for the non-trivial
//! bridges, to a future `objc2`-based implementation). Bodies are marked
//! `unimplemented!()` until those bridges are wired up. On every other
//! platform the module exposes empty stubs so the file still compiles
//! inside a `pcsx2_translations` library.

#![allow(dead_code)]

// Opaque handle types that mirror the C++ side. We deliberately do not
// try to model `WindowInfo` here; callers pass it through as a raw
// pointer exactly as the original header does, preserving ABI.
#[cfg(target_os = "macos")]
#[repr(C)]
pub struct WindowInfo {
    _private: [u8; 0],
}

/// Returns `true` when the build target is macOS.
///
/// Mirrors the `__APPLE__` guard in the C++ header.
#[cfg(target_os = "macos")]
#[inline]
pub const fn is_macos() -> bool {
    true
}

/// Stub used on non-macOS targets so cross-platform code can
/// reference the same symbol uniformly.
#[cfg(not(target_os = "macos"))]
#[inline]
pub const fn is_macos() -> bool {
    false
}

// ---------------------------------------------------------------------------
// macOS implementation
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod macos {
    use super::WindowInfo;
    use std::ffi::CStr;
    use std::os::raw::c_char;

    /// `bool` from C, modelled as `i8` for FFI safety (matches the
    /// representation used by clang on Apple platforms).
    type CBool = i8;

    /// Link against the system Foundation / AppKit / Metal framework
    /// collection that the original `CocoaTools.mm` translation unit
    /// pulls in. Names are the canonical `*-framework` aliases.
    #[link(name = "Foundation", kind = "framework")]
    #[link(name = "AppKit", kind = "framework")]
    #[link(name = "Metal", kind = "framework")]
    extern "C" {}

    // -- Window / Metal layer helpers -----------------------------------

    /// Attach a `CAMetalLayer` to the `NSView` described by `wi`.
    ///
    /// Returns `true` on success. Mirrors `CocoaTools::CreateMetalLayer`.
    #[inline]
    pub extern "C" fn CreateMetalLayer(wi: *mut WindowInfo) -> CBool {
        // TODO: bridge to Objective-C via the existing CocoaTools.mm
        // implementation, or rewrite using `objc2`.
        let _ = wi;
        unimplemented!("CreateMetalLayer: Objective-C bridge not yet wired")
    }

    /// Detach and release the `CAMetalLayer` previously attached to
    /// the `NSView` described by `wi`. Mirrors
    /// `CocoaTools::DestroyMetalLayer`.
    #[inline]
    pub extern "C" fn DestroyMetalLayer(wi: *mut WindowInfo) {
        // TODO: bridge to Objective-C.
        let _ = wi;
        unimplemented!("DestroyMetalLayer: Objective-C bridge not yet wired")
    }

    /// Returns the display refresh rate associated with the `NSView`
    /// described by `wi`, in Hertz. `None` if the rate cannot be
    /// determined. Mirrors `CocoaTools::GetViewRefreshRate`.
    #[inline]
    pub extern "C" fn GetViewRefreshRate(wi: *const WindowInfo) -> Option<f32> {
        // TODO: bridge to Objective-C; the C++ version reads
        // `NSScreen.maximumRefreshRate` and walks the display link.
        let _ = wi;
        unimplemented!("GetViewRefreshRate: Objective-C bridge not yet wired")
    }

    // -- Menu / bundle / file helpers -----------------------------------

    /// Tag the given `NSMenu` (passed as an opaque `void*`) as the
    /// application's help menu. Mirrors `CocoaTools::MarkHelpMenu`.
    #[inline]
    pub extern "C" fn MarkHelpMenu(menu: *mut std::ffi::c_void) {
        // TODO: bridge to Objective-C; equivalent to
        // `[NSApp setHelpMenu:(__kindof NSMenu*)menu]`.
        let _ = menu;
        unimplemented!("MarkHelpMenu: Objective-C bridge not yet wired")
    }

    /// Returns the bundle path of the running application.
    /// Mirrors `CocoaTools::GetBundlePath`.
    #[inline]
    pub extern "C" fn GetBundlePath() -> Option<String> {
        // TODO: bridge to Objective-C; equivalent to
        // `[[NSBundle mainBundle] bundlePath]`.
        unimplemented!("GetBundlePath: Objective-C bridge not yet wired")
    }

    /// Returns the bundle path of the running application, skipping any
    /// macOS translocation. Mirrors
    /// `CocoaTools::GetNonTranslocatedBundlePath`.
    #[inline]
    pub extern "C" fn GetNonTranslocatedBundlePath() -> Option<String> {
        // TODO: bridge to Objective-C; equivalent to walking
        // `SecTranslocateIsTranslocatedURL` + the original path.
        unimplemented!("GetNonTranslocatedBundlePath: Objective-C bridge not yet wired")
    }

    /// Move the file at `file` to the trash and return its new path.
    /// Mirrors `CocoaTools::MoveToTrash`.
    #[inline]
    pub extern "C" fn MoveToTrash(file: &str) -> Option<String> {
        // TODO: bridge to Objective-C; equivalent to
        // `[[NSFileManager defaultManager] trashItemAtURL:...].
        let _ = file;
        unimplemented!("MoveToTrash: Objective-C bridge not yet wired")
    }

    /// Schedule `file` to be launched once this application quits.
    /// Mirrors `CocoaTools::DelayedLaunch`.
    #[inline]
    pub extern "C" fn DelayedLaunch(file: &str) -> bool {
        // TODO: bridge to Objective-C; the C++ version uses
        // `LSRegisterURL` + `LSOpenURLsWithRole`.
        let _ = file;
        unimplemented!("DelayedLaunch: Objective-C bridge not yet wired")
    }

    /// Open a Finder window pointing at `file`. Mirrors
    /// `CocoaTools::ShowInFinder`.
    #[inline]
    pub extern "C" fn ShowInFinder(file: &str) -> bool {
        // TODO: bridge to Objective-C; equivalent to
        // `[[NSWorkspace sharedWorkspace] selectFile:... inFileViewerRootedAtPath:@""]
        let _ = file;
        unimplemented!("ShowInFinder: Objective-C bridge not yet wired")
    }

    /// Returns the path to the `Resources` directory of the current
    /// application bundle. Mirrors `CocoaTools::GetResourcePath`.
    #[inline]
    pub extern "C" fn GetResourcePath() -> Option<String> {
        // TODO: bridge to Objective-C; equivalent to
        // `[[NSBundle mainBundle] resourcePath]`.
        unimplemented!("GetResourcePath: Objective-C bridge not yet wired")
    }

    // -- Window lifecycle helpers ---------------------------------------

    /// Create a new `NSWindow` with the given title and pixel size, and
    /// return it as an opaque pointer. Mirrors
    /// `CocoaTools::CreateWindow`.
    #[inline]
    pub extern "C" fn CreateWindow(
        title: &CStr,
        width: u32,
        height: u32,
    ) -> *mut std::ffi::c_void {
        // TODO: bridge to Objective-C; equivalent to allocating an
        // `NSWindow` + `NSView` pair and returning a retained pointer.
        let _ = (title, width, height);
        unimplemented!("CreateWindow: Objective-C bridge not yet wired")
    }

    /// Release a window previously returned by `CreateWindow`.
    /// Mirrors `CocoaTools::DestroyWindow`.
    #[inline]
    pub extern "C" fn DestroyWindow(window: *mut std::ffi::c_void) {
        // TODO: bridge to Objective-C; releases the `NSWindow` via
        // `objc_msgSend`-style call.
        let _ = window;
        unimplemented!("DestroyWindow: Objective-C bridge not yet wired")
    }

    /// Populate the `WindowInfo` pointed to by `wi` from the `NSWindow`
    /// pointed to by `window`. Mirrors
    /// `CocoaTools::GetWindowInfoFromWindow`.
    #[inline]
    pub extern "C" fn GetWindowInfoFromWindow(
        wi: *mut WindowInfo,
        window: *mut std::ffi::c_void,
    ) {
        // TODO: bridge to Objective-C; the C++ version copies the
        // window's frame, scale factor, and content view handle into
        // the `WindowInfo` struct.
        let _ = (wi, window);
        unimplemented!("GetWindowInfoFromWindow: Objective-C bridge not yet wired")
    }

    /// Run the Cocoa event loop. When `wait_forever` is `true` the
    /// loop will not return until `StopMainThreadEventLoop` is called.
    /// Mirrors `CocoaTools::RunCocoaEventLoop`.
    #[inline]
    pub extern "C" fn RunCocoaEventLoop(wait_forever: bool) {
        // TODO: bridge to Objective-C; equivalent to driving
        // `[NSApp run]` / `[NSApp runUntilDate:...]` accordingly.
        let _ = wait_forever;
        unimplemented!("RunCocoaEventLoop: Objective-C bridge not yet wired")
    }

    /// Post an event to the main thread that causes a pending
    /// `RunCocoaEventLoop(true)` to return. Mirrors
    /// `CocoaTools::StopMainThreadEventLoop`.
    #[inline]
    pub extern "C" fn StopMainThreadEventLoop() {
        // TODO: bridge to Objective-C; the C++ version posts an
        // `NSEventTypeApplicationDefined` event to `[NSApp stop]`.
        unimplemented!("StopMainThreadEventLoop: Objective-C bridge not yet wired")
    }

    // Allow the unused-type lint: `c_char` is referenced for
    // documentation parity with the original header's C-string
    // parameters.
    #[allow(dead_code)]
    fn _phantom_c_char(_: *const c_char) {}
}

#[cfg(target_os = "macos")]
pub use macos::*;

// ---------------------------------------------------------------------------
// Non-macOS stubs
// ---------------------------------------------------------------------------
//
// Every public symbol from the macOS side has a non-macOS counterpart so
// that cross-platform code can write `CocoaTools::CreateMetalLayer(...)`
// (or `crate::common::CocoaTools::CreateMetalLayer`) regardless of the
// build target. On non-macOS the functions are no-ops that return the
// "no value" / `false` equivalents of their C++ counterparts.

/// Stub: no-op on non-macOS. See `macos::CreateMetalLayer`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn CreateMetalLayer(_wi: *mut std::ffi::c_void) -> bool {
    false
}

/// Stub: no-op on non-macOS. See `macos::DestroyMetalLayer`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn DestroyMetalLayer(_wi: *mut std::ffi::c_void) {}

/// Stub: always returns `None` off macOS. See
/// `macos::GetViewRefreshRate`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn GetViewRefreshRate(_wi: *const std::ffi::c_void) -> Option<f32> {
    None
}

/// Stub: no-op on non-macOS. See `macos::MarkHelpMenu`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn MarkHelpMenu(_menu: *mut std::ffi::c_void) {}

/// Stub: always returns `None` off macOS. See `macos::GetBundlePath`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn GetBundlePath() -> Option<String> {
    None
}

/// Stub: always returns `None` off macOS. See
/// `macos::GetNonTranslocatedBundlePath`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn GetNonTranslocatedBundlePath() -> Option<String> {
    None
}

/// Stub: always returns `None` off macOS. See `macos::MoveToTrash`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn MoveToTrash(_file: &str) -> Option<String> {
    None
}

/// Stub: always returns `false` off macOS. See `macos::DelayedLaunch`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn DelayedLaunch(_file: &str) -> bool {
    false
}

/// Stub: always returns `false` off macOS. See `macos::ShowInFinder`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn ShowInFinder(_file: &str) -> bool {
    false
}

/// Stub: always returns `None` off macOS. See `macos::GetResourcePath`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn GetResourcePath() -> Option<String> {
    None
}

/// Stub: returns a null pointer off macOS. See `macos::CreateWindow`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn CreateWindow(
    _title: &str,
    _width: u32,
    _height: u32,
) -> *mut std::ffi::c_void {
    std::ptr::null_mut()
}

/// Stub: no-op on non-macOS. See `macos::DestroyWindow`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn DestroyWindow(_window: *mut std::ffi::c_void) {}

/// Stub: no-op on non-macOS. See `macos::GetWindowInfoFromWindow`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn GetWindowInfoFromWindow(
    _wi: *mut std::ffi::c_void,
    _window: *mut std::ffi::c_void,
) {
}

/// Stub: no-op on non-macOS. See `macos::RunCocoaEventLoop`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn RunCocoaEventLoop(_wait_forever: bool) {}

/// Stub: no-op on non-macOS. See `macos::StopMainThreadEventLoop`.
#[cfg(not(target_os = "macos"))]
#[inline]
pub fn StopMainThreadEventLoop() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_flag_matches_target() {
        assert_eq!(is_macos(), cfg!(target_os = "macos"));
    }

    #[test]
    fn non_macos_stubs_return_sensible_defaults() {
        // On every target the stubs below should produce the
        // documented defaults without panicking.
        assert!(!CreateMetalLayer(std::ptr::null_mut()));
        assert!(GetViewRefreshRate(std::ptr::null()).is_none());
        assert!(GetBundlePath().is_none());
        assert!(GetNonTranslocatedBundlePath().is_none());
        assert!(MoveToTrash("foo").is_none());
        assert!(!DelayedLaunch("foo"));
        assert!(!ShowInFinder("foo"));
        assert!(GetResourcePath().is_none());
        assert!(CreateWindow("title", 100, 100).is_null());

        DestroyMetalLayer(std::ptr::null_mut());
        MarkHelpMenu(std::ptr::null_mut());
        DestroyWindow(std::ptr::null_mut());
        GetWindowInfoFromWindow(std::ptr::null_mut(), std::ptr::null_mut());
        RunCocoaEventLoop(false);
        StopMainThreadEventLoop();
    }
}
