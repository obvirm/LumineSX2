// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! macOS-only Objective-C wrappers for window management and event loops.
//!
//! Rust port of `common/CocoaTools.{h,mm}`. Compiles to empty stubs on
//! every non-macOS target so callers can be platform-agnostic.
//!
//! # Cargo.toml dependencies (macOS only)
//!
//! Add the following to `rust/common/Cargo.toml` under a target
//! section (these are doc-only here; the crate's own `Cargo.toml` is
//! not modified by this translation):
//!
//! ```toml
//! [target.'cfg(target_os = "macos")'.dependencies]
//! cocoa = "0.25"
//! objc  = "0.2"
//! ```
//!
//! # Conventions
//!
//! - Opaque `*mut c_void` handles represent retained Objective-C
//!   objects (currently `NSWindow`). The destructor is `destroy_window`.
//! - All UI work must happen on the main thread, mirroring AppKit's
//!   threading rules. The C++ source had the same restriction but did
//!   not enforce it.
//! - The C++ side calls into Rust via the `pcsx2_cocoa_*` FFI exports
//!   below; the pure-Rust `create_window` / `destroy_window` /
//!   `run_event_loop` / `stop_event_loop` are exposed too for internal
//!   use.

#![cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]

use std::os::raw::{c_char, c_void};

// =====================================================================
// Platform-specific implementation
// =====================================================================

#[cfg(target_os = "macos")]
mod imp {
    //! macOS implementation backed by AppKit via the `cocoa` and
    //! `objc` crates.
    use super::*;
    use cocoa::appkit::{
        NSApp, NSApplication, NSApplicationActivationPolicy, NSBackingStoreBuffered,
        NSWindowStyleMask,
    };
    use cocoa::base::{id, nil, NO, YES};
    use cocoa::foundation::{NSAutoreleasePool, NSPoint, NSRect, NSSize, NSString};
    use objc::{class, msg_send, sel, sel_impl};

    /// Opaque handle mirroring the C++ `void*` (ARC-retained
    /// `NSWindow*`).
    pub type WindowHandle = *mut c_void;

    /// Subtype used by the sentinel "stop event loop" `NSEvent`.
    const STOP_EVENT_LOOP: i16 = 0x100;

    /// `NSEventTypeApplicationDefined == 15` (Cocoa header constant).
    const NS_EVENT_TYPE_APPLICATION_DEFINED: u64 = 15;
    /// `NSEventMaskAny` — accepts every event mask (`UINT64_MAX`).
    const NS_EVENT_MASK_ANY: u64 = u64::MAX;

    /// Initialise `NSApp` if it has not been created yet, set the
    /// activation policy to `.Regular` and finish launching. Safe to
    /// call repeatedly: `setActivationPolicy_` and `finishLaunching`
    /// are both idempotent.
    unsafe fn ensure_app() -> id {
        let app = NSApp();
        app.setActivationPolicy_(
            NSApplicationActivationPolicy::NSApplicationActivationPolicyRegular,
        );
        let _: () = msg_send![app, finishLaunching];
        app
    }

    /// Create a centred, key `NSWindow` with the given title and size.
    ///
    /// Mirrors `CocoaTools::CreateWindow`. The window is created with
    /// the standard titled/closable/miniaturisable/resizable style
    /// mask and a buffered backing store. The returned pointer is an
    /// ARC-retained `NSWindow*`; call [`destroy_window`] to release
    /// it.
    pub fn create_window(title: &str, width: u32, height: u32) -> WindowHandle {
        unsafe {
            let _pool = NSAutoreleasePool::new(nil);
            let _app = ensure_app();

            // Centre the new window on the main screen.
            let screen: id = msg_send![class!(NSScreen), mainScreen];
            let screen_frame: NSRect = msg_send![screen, frame];
            let mut view_frame = screen_frame;
            view_frame.size = NSSize::new(width as f64, height as f64);
            view_frame.origin.x += (screen_frame.size.width - view_frame.size.width) / 2.0;
            view_frame.origin.y += (screen_frame.size.height - view_frame.size.height) / 2.0;

            // Standard titled/closable/miniaturisable/resizable window.
            let style = NSWindowStyleMask::NSWindowStyleMaskTitled
                | NSWindowStyleMask::NSWindowStyleMaskClosable
                | NSWindowStyleMask::NSWindowStyleMaskMiniaturizable
                | NSWindowStyleMask::NSWindowStyleMaskResizable;

            // alloc + initWithContentRect:styleMask:backing:defer:
            let window_alloc: id = msg_send![class!(NSWindow), alloc];
            let window: id = msg_send![
                window_alloc,
                initWithContentRect: view_frame
                              styleMask: style
                                backing: NSBackingStoreBuffered::NSBackingStoreBuffered
                                  defer: NO
            ];
            if window == nil {
                return std::ptr::null_mut();
            }

            // Title.
            let ns_title = NSString::alloc(nil).init_str(title);
            let _: () = msg_send![window, setTitle: ns_title];

            // Show on screen.
            let _: () = msg_send![window, makeKeyAndOrderFront: window];

            // Bump the retain count so the caller's release in
            // `destroy_window` is balanced. This mirrors the C++
            // `__bridge_retained` cast in `CocoaTools::CreateWindow`.
            let _: id = msg_send![window, retain];
            window as WindowHandle
        }
    }

    /// Release a window handle previously returned by [`create_window`].
    ///
    /// Mirrors `CocoaTools::DestroyWindow`'s `__bridge_transfer` cast.
    /// Safe to call with a null pointer.
    pub fn destroy_window(window: WindowHandle) {
        if window.is_null() {
            return;
        }
        unsafe {
            let _: () = msg_send![window as id, release];
        }
    }

    /// Drain the Cocoa event loop on the calling thread.
    ///
    /// When `forever` is true the call only returns once
    /// [`stop_event_loop`] has posted its sentinel `NSEvent`. When
    /// `false` it drains all currently pending events and returns.
    /// Mirrors `CocoaTools::RunCocoaEventLoop`.
    pub fn run_event_loop(forever: bool) {
        unsafe {
            let app = ensure_app();
            let end: id = if forever {
                msg_send![class!(NSDate), distantFuture]
            } else {
                msg_send![class!(NSDate), distantPast]
            };

            loop {
                let pool: id = msg_send![class!(NSAutoreleasePool), new];
                let ev: id = msg_send![
                    app,
                    nextEventMatchingMask: NS_EVENT_MASK_ANY
                                    untilDate: end
                                       inMode: cocoa::foundation::NSDefaultRunLoopMode
                                      dequeue: YES
                ];
                if ev == nil {
                    let _: () = msg_send![pool, drain];
                    break;
                }
                let ev_type: u64 = msg_send![ev, type];
                let subtype: i16 = msg_send![ev, subtype];
                if ev_type == NS_EVENT_TYPE_APPLICATION_DEFINED
                    && subtype == STOP_EVENT_LOOP
                {
                    let _: () = msg_send![pool, drain];
                    break;
                }
                let _: () = msg_send![app, sendEvent: ev];
                let _: () = msg_send![pool, drain];
            }
        }
    }

    /// Post the sentinel `NSEvent` that causes a `run_event_loop(true)`
    /// call to return. Mirrors `CocoaTools::StopMainThreadEventLoop`.
    pub fn stop_event_loop() {
        unsafe {
            let app = NSApp();
            let ev: id = msg_send![
                class!(NSEvent),
                otherEventWithType: NS_EVENT_TYPE_APPLICATION_DEFINED
                                location: NSPoint::new(0.0, 0.0)
                           modifierFlags: 0u64
                               timestamp: 0.0_f64
                            windowNumber: 0_isize
                                 context: nil as id
                                 subtype: STOP_EVENT_LOOP
                                   data1: 0_isize
                                   data2: 0_isize
            ];
            if ev == nil {
                return;
            }
            let _: () = msg_send![app, postEvent: ev atStart: NO];
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    //! No-op stubs for non-macOS platforms.
    use super::*;
    pub type WindowHandle = *mut c_void;
    pub fn create_window(_title: &str, _width: u32, _height: u32) -> WindowHandle {
        std::ptr::null_mut()
    }
    pub fn destroy_window(_window: WindowHandle) {}
    pub fn run_event_loop(_forever: bool) {}
    pub fn stop_event_loop() {}
}

// =====================================================================
// Public Rust API (re-exported on every platform)
// =====================================================================

pub use imp::*;

// =====================================================================
// C ABI exports (consumed by the C++ side via cbindgen)
// =====================================================================

/// Create an `NSWindow` with the given title, width and height. The
/// returned pointer is an opaque, owned handle that must be released
/// with `pcsx2_cocoa_destroy_window`. Returns null on non-macOS
/// platforms or on failure.
#[no_mangle]
pub extern "C" fn pcsx2_cocoa_create_window(
    title: *const c_char,
    width: u32,
    height: u32,
) -> *mut c_void {
    let title_str = if title.is_null() {
        ""
    } else {
        // Safety: caller guarantees a NUL-terminated C string.
        match unsafe { std::ffi::CStr::from_ptr(title) }.to_str() {
            Ok(s) => s,
            Err(_) => "",
        }
    };
    create_window(title_str, width, height)
}

/// Release a window handle previously returned by
/// `pcsx2_cocoa_create_window`. Safe to call with a null pointer.
#[no_mangle]
pub extern "C" fn pcsx2_cocoa_destroy_window(window: *mut c_void) {
    destroy_window(window);
}

/// Run the Cocoa event loop on the calling thread until
/// `pcsx2_cocoa_stop_loop` is called. No-op on non-macOS platforms.
#[no_mangle]
pub extern "C" fn pcsx2_cocoa_run_loop() {
    run_event_loop(true);
}

/// Post the sentinel event that terminates a `pcsx2_cocoa_run_loop`
/// call. No-op on non-macOS platforms.
#[no_mangle]
pub extern "C" fn pcsx2_cocoa_stop_loop() {
    stop_event_loop();
}
