// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Rust reimplementation of PCSX2's `common/Darwin/DarwinMisc.{h,cpp}`.
//!
//! macOS-specific desktop integration helpers:
//!
//! - [`inhibit_screensaver`] — prevent the user-idle display sleep
//!   assertion from kicking in while a game is running. Implemented
//!   via `IOPMAssertionCreateWithName` / `IOPMAssertionRelease` from
//!   IOKit's power-management library.
//! - [`play_sound_async`] — fire-and-forget playback of an audio file
//!   via AppKit's `NSSound`.
//! - [`set_mouse_position`] — warp the OS cursor to a global screen
//!   coordinate using Core Graphics' `CGWarpMouseCursorPosition`. We
//!   also briefly disassociate the mouse from the cursor
//!   (`CGAssociateMouseAndMouseCursorPosition`) so the warp is not
//!   undone by the next user move, exactly as the C++ original does.
//! - [`get_program_path`] — return the absolute path of the running
//!   executable via `_NSGetExecutablePath`.
//! - [`set_path_compression`] — no-op on macOS. The C++ `HostSys`
//!   surface has no equivalent on Darwin; the function exists only to
//!   satisfy the cross-platform header.
//!
//! # Dependencies
//!
//! This module needs macOS-only crates. Add the following to
//! `Cargo.toml` under `[target.'cfg(target_os = "macos")'.dependencies]`:
//!
//! ```toml
//! cocoa = "0.25"
//! objc = "0.2"
//! core-graphics = "0.23"
//! ```
//!
//! `cocoa` gives us the AppKit (`NSURL`, `NSString`) bindings, `objc`
//! is the raw Objective-C runtime we use to send
//! `initWithContentsOfURL:byRef:` and `-play` to the `NSSound` class,
//! and `core-graphics` provides the `CGWarpMouseCursorPosition` /
//! `CGAssociateMouseAndMouseCursorPosition` surface used by
//! [`set_mouse_position`]. The IOKit power-management declarations
//! are declared inline in `imp::iokit` below because `io-kit-sys`
//! does not currently expose `IOPMAssertion*` symbols on crates.io.
//!
//! # FFI surface
//!
//! Each pure-Rust entry point has a matching `#[no_mangle] pub extern
//! "C" fn pcsx2_macos_*` wrapper for the C++ core to call through
//! `cbindgen`. Strings cross the boundary as `*const c_char`; the C++
//! side owns the lifetime.

#![cfg(target_os = "macos")]
#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    clippy::all,
)]

use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use cocoa::base::{id, nil, BOOL, YES};
use cocoa::foundation::NSString;
use objc::{class, msg_send, sel, sel_impl};

// ---------------------------------------------------------------------------
// IOKit power management
//
// `IOPMAssertion*` is not in the macOS SDK's libsystem; it lives in
// IOKit.framework. Declaring the few symbols we need here keeps the
// crate's dependency footprint small (no `io-kit-sys`).
// ---------------------------------------------------------------------------

mod imp {
    use std::os::raw::{c_char, c_void};

    /// Opaque assertion identifier returned by `IOPMAssertionCreateWithName`.
    pub type IOPMAssertionID = u32;

    /// Asserts preventing the display from sleeping on user idle. The
    /// C++ side passes `kIOPMAssertionTypePreventUserIdleDisplaySleep`
    /// (which has the literal value "PreventUserIdleDisplaySleep" as
    /// a CFString).
    #[link(name = "IOKit", kind = "framework")]
    extern "C" {
        pub fn IOPMAssertionCreateWithName(
            assertion_type: *const c_void,
            level: i32,
            name: *const c_void,
            assertion_id: *mut IOPMAssertionID,
        ) -> i32;

        pub fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> i32;
    }

    /// Wrap the `CFStringCreateWithCString` entry point so we can
    /// build `CFStringRef`s for the assertion type / reason.
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const c_char,
            encoding: u32,
        ) -> *const c_void;
    }

    /// kCFStringEncodingUTF8 == 0x08000100. Hard-coded to avoid
    /// pulling in another framework constant module.
    pub const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

    /// `CFRelease` is the matching deallocator for everything
    /// `CF*Create*` returns.
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        pub fn CFRelease(cf: *const c_void);
    }
}

// ---------------------------------------------------------------------------
// Module-local state
//
// `IOPMAssertionID` is a process-wide singleton in the C++ version
// (`static IOPMAssertionID s_pm_assertion`). We model it with an
// `AtomicU32` so concurrent FFI callers can read/write the held id
// without UB; the only atomicity we actually need is "no torn
// reads" — a torn write would have been a torn write of an
// `IOPMAssertionID` in C++ too. `AcqRel` is sufficient because the
// IOKit call itself is the linearisation point.
// ---------------------------------------------------------------------------

/// The currently held power-management assertion id, or `0` when no
/// assertion is held. Mirrors `static IOPMAssertionID s_pm_assertion`.
static HELD_ASSERTION: AtomicU32 = AtomicU32::new(0);

/// Human-readable label attached to the assertion when we hold one.
/// Mirrors the C++ `CFSTR("Playing a game")` literal.
const ASSERTION_REASON: &str = "Playing a game";

// ---------------------------------------------------------------------------
// Pure-Rust API
// ---------------------------------------------------------------------------

/// Prevent the macOS screen saver / display sleep while `inhibit` is
/// `true`, release the assertion otherwise.
///
/// Mirrors `Common::InhibitScreensaver`. Returns `true` if the call
/// succeeded. The C++ original unconditionally returns `true`; we
/// surface the underlying IOKit error so callers can detect failure
/// (e.g. when running under a daemon that has no power-management
/// privileges).
///
/// # Implementation
///
/// The first thing we do is release whatever assertion we currently
/// hold. This makes the function idempotent — toggling `inhibit` on
/// and off without a paired call cannot leak a held assertion.
///
/// When `inhibit` is `true` we create a new assertion of type
/// `kIOPMAssertionTypePreventUserIdleDisplaySleep` and stash the id
/// in [`HELD_ASSERTION`] for a later release.
pub fn inhibit_screensaver(inhibit: bool) -> bool {
    let prev = HELD_ASSERTION.swap(0, Ordering::AcqRel);
    if prev != 0 {
        // SAFETY: `prev` came from `IOPMAssertionCreateWithName` so it
        // is valid to release.
        let rc = unsafe { imp::IOPMAssertionRelease(prev) };
        if rc != 0 {
            // Restore on failure so the caller can retry; otherwise
            // we'd lose the assertion without actually releasing it.
            HELD_ASSERTION.store(prev, Ordering::Release);
            return false;
        }
    }

    if !inhibit {
        return true;
    }

    let assertion_type = match cfstring("PreventUserIdleDisplaySleep") {
        Some(s) => s,
        None => return false,
    };
    let assertion_name = match cfstring(ASSERTION_REASON) {
        Some(s) => s,
        None => {
            unsafe { imp::CFRelease(assertion_type) };
            return false;
        }
    };

    let mut id: imp::IOPMAssertionID = 0;
    // SAFETY: both CFStrings are freshly allocated and the `id`
    // pointer is a valid out-parameter. `IOPMAssertionCreateWithName`
    // returns `kIOReturnSuccess` (0) on success.
    let rc = unsafe {
        imp::IOPMAssertionCreateWithName(
            assertion_type,
            /* IOPMAssertionLevel::On */ 1,
            assertion_name,
            &mut id,
        )
    };
    // Free the CFStrings we allocated — the power-management call
    // copies them internally.
    unsafe { imp::CFRelease(assertion_type) };
    unsafe { imp::CFRelease(assertion_name) };

    if rc != 0 {
        return false;
    }
    HELD_ASSERTION.store(id, Ordering::Release);
    true
}

/// Asynchronously play the audio file at `path`.
///
/// Mirrors `Common::PlaySoundAsync`. Returns `true` if AppKit
/// successfully loaded the file and started playback.
///
/// We construct an `NSURL` from the file path, hand it to
/// `[[NSSound alloc] initWithContentsOfURL:byRef:]`, and call
/// `-play` on the resulting instance. The `NSSound` reference is
/// released after `play` (which retains internally for the duration
/// of playback).
pub fn play_sound_async(path: &Path) -> bool {
    let path_str = match path.to_str() {
        Some(s) => s,
        None => return false,
    };

    // SAFETY: `NSString` class is always available and
    // `stringWithUTF8String:` returns null on invalid UTF-8 (which
    // we already checked above).
    let ns_path = unsafe { NSString::alloc(nil).init_str(path_str) };
    if ns_path.is_null() {
        return false;
    }

    // `[NSURL fileURLWithPath:]` is the documented way to build a
    // file URL. We use the alloc/init pattern via objc message sends
    // because `cocoa` does not expose `NSURL` directly.
    let ns_url_class = class!(NSURL);
    let url_alloc: id = unsafe { msg_send![ns_url_class, alloc] };
    // SAFETY: `url_alloc` is a freshly-allocated `NSURL`; the
    // selector takes a single `NSString*` and returns an `id`.
    let ns_url: id = unsafe { msg_send![url_alloc, initFileURLWithPath: ns_path] };
    if ns_url.is_null() {
        return false;
    }

    // `[[NSSound alloc] initWithContentsOfURL:byRef:]`. We use
    // `byRef:YES` so AppKit keeps the file handle alive for the
    // duration of playback (the C++ original's behaviour).
    let ns_sound_class = class!(NSSound);
    let sound_alloc: id = unsafe { msg_send![ns_sound_class, alloc] };
    let by_ref: BOOL = YES;
    // SAFETY: `sound_alloc` is a freshly-allocated `NSSound`; the
    // selector takes an `NSURL*` and a `BOOL` and returns an `id`.
    let sound: id = unsafe { msg_send![sound_alloc, initWithContentsOfURL: ns_url byRef: by_ref] };
    if sound.is_null() {
        return false;
    }

    // `-play` is fire-and-forget; the resulting `NSSound` retains
    // itself for the duration of playback and releases when done.
    let _: () = unsafe { msg_send![sound, play] };

    // We held the alloc-returned reference; release it now that
    // `-play` has finished setting up.
    let _: () = unsafe { msg_send![sound, release] };

    true
}

/// Warp the OS mouse cursor to the global screen coordinate `(x, y)`.
///
/// Mirrors `Common::SetMousePosition`. The C++ implementation calls
/// `CGAssociateMouseAndMouseCursorPosition(false)` before
/// `CGWarpMouseCursorPosition` and resets it to `true` afterwards so
/// that the warp isn't immediately undone by the next mouse move. We
/// do the same here, with `core-graphics`'s `display` module taking
/// care of the binding.
pub fn set_mouse_position(x: i32, y: i32) {
    use core_graphics::display::{associate_mouse_and_mouse_cursor_position, warp_mouse_cursor_position};
    use core_graphics::geometry::CGPoint;

    // `CGAssociateMouseAndMouseCursorPosition(false)` — `false`
    // disassociates the cursor from the mouse so the warp sticks.
    associate_mouse_and_mouse_cursor_position(false);

    // SAFETY: `CGWarpMouseCursorPosition` is a simple void function
    // that takes a `CGPoint`. `core-graphics` exposes it as a safe
    // wrapper so there is no `unsafe` block here.
    warp_mouse_cursor_position(CGPoint::new(x as f64, y as f64));

    // `CGAssociateMouseAndMouseCursorPosition(true)` — restore the
    // default state so user-driven mouse moves work again.
    associate_mouse_and_mouse_cursor_position(true);
}

/// Return the absolute path of the currently running executable.
///
/// Mirrors `GetProgramPath`. We call `_NSGetExecutablePath` which
/// copies the path into a caller-supplied buffer (sized via
/// `PATH_MAX` plus a retry loop in case the buffer is too small).
///
/// On error — extremely rare, only if `PATH_MAX` cannot be queried
/// or the path genuinely doesn't fit — we fall back to the current
/// working directory so callers always receive a usable path.
pub fn get_program_path() -> PathBuf {
    const PATH_MAX: usize = 4096;

    extern "C" {
        fn _NSGetExecutablePath(buf: *mut c_char, bufsize: *mut u32) -> i32;
    }

    let mut buf = [0i8; PATH_MAX];
    let mut size = buf.len() as u32;
    // SAFETY: `buf` is a writable stack array of `PATH_MAX` bytes;
    // `_NSGetExecutablePath` writes up to `size` bytes including the
    // NUL terminator and overwrites `size` with the required length
    // when the buffer is too small.
    let rc = unsafe { _NSGetExecutablePath(buf.as_mut_ptr(), &mut size) };
    if rc == 0 {
        // `size` now holds the number of bytes written including the
        // terminator; trim the trailing NUL for `CStr::from_ptr`.
        let cstr = unsafe { CStr::from_ptr(buf.as_ptr()) };
        return PathBuf::from(cstr.to_string_lossy().as_ref());
    }

    // Buffer was too small; `size` has been updated. Allocate the
    // required amount and retry.
    if size > 0 {
        let mut owned = vec![0i8; size as usize];
        let mut retry_size = size;
        // SAFETY: `owned` is freshly allocated with `size` writable
        // bytes; `_NSGetExecutablePath` writes up to that and updates
        // `retry_size` with the actual length.
        let rc2 = unsafe { _NSGetExecutablePath(owned.as_mut_ptr(), &mut retry_size) };
        if rc2 == 0 {
            let cstr = unsafe { CStr::from_ptr(owned.as_ptr()) };
            return PathBuf::from(cstr.to_string_lossy().as_ref());
        }
    }

    // Last-ditch fallback. Returning `"."` keeps callers happy
    // without lying about the path.
    PathBuf::from(".")
}

/// Stub used to satisfy the cross-platform `HostSys::SetPathCompression`
/// surface on macOS. Path compression (the Windows Bridge-style
/// `\\?\` extended-length path prefix) has no analogue on Darwin, so
/// this is unconditionally a no-op returning `false` (the C++ original
/// returns `true` on Windows; we pick `false` to signal "no change
/// was made").
pub fn set_path_compression(_path: &mut PathBuf) -> bool {
    let _ = _path;
    false
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Wrap the C `CFSTR(...)` macro. Builds a `CFStringRef` from a Rust
/// `&str` using `CFStringCreateWithCString` and the UTF-8 encoding.
///
/// Returns null (represented here as `None`) on allocation failure.
/// Callers are responsible for releasing the returned string with
/// `imp::CFRelease`.
fn cfstring(s: &str) -> Option<*const c_void> {
    let cstr = match std::ffi::CString::new(s) {
        Ok(c) => c,
        Err(_) => return None,
    };
    // SAFETY: `cstr` is a valid NUL-terminated UTF-8 C string.
    // `kCFAllocatorDefault` is null on macOS — pass null explicitly.
    let p = unsafe {
        imp::CFStringCreateWithCString(
            std::ptr::null(),
            cstr.as_ptr(),
            imp::K_CF_STRING_ENCODING_UTF8,
        )
    };
    if p.is_null() {
        None
    } else {
        Some(p)
    }
}

// ---------------------------------------------------------------------------
// FFI surface
// ---------------------------------------------------------------------------

/// FFI export of [`inhibit_screensaver`].
#[no_mangle]
pub extern "C" fn pcsx2_macos_inhibit_screensaver(inhibit: bool) -> bool {
    inhibit_screensaver(inhibit)
}

/// FFI export of [`play_sound_async`]. `path` must be a NUL-terminated
/// UTF-8 C string; passing null returns `false`.
#[no_mangle]
pub extern "C" fn pcsx2_macos_play_sound_async(path: *const c_char) -> bool {
    if path.is_null() {
        return false;
    }
    // SAFETY: caller guarantees `path` is a valid NUL-terminated
    // C string and remains valid for the duration of this call.
    let cstr = unsafe { CStr::from_ptr(path) };
    let owned = match cstr.to_str() {
        Ok(s) => PathBuf::from(s),
        Err(_) => return false,
    };
    play_sound_async(&owned)
}

/// FFI export of [`set_mouse_position`].
#[no_mangle]
pub extern "C" fn pcsx2_macos_set_mouse_position(x: i32, y: i32) {
    set_mouse_position(x, y);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// `set_path_compression` is a no-op on macOS and must always
    /// return `false`. The argument is intentionally mutated outside
    /// the call to verify the function does not read or write it.
    #[test]
    fn set_path_compression_is_noop() {
        let mut p = PathBuf::from("/tmp/example");
        let original = p.clone();
        assert!(!set_path_compression(&mut p));
        assert_eq!(p, original, "set_path_compression must not touch its argument");
    }

    /// `cfstring` returns a non-null pointer for a valid UTF-8 input
    /// and null for inputs containing interior NULs.
    #[test]
    fn cfstring_roundtrip() {
        let p = cfstring("PreventUserIdleDisplaySleep").expect("valid CFString");
        assert!(!p.is_null());
        unsafe { imp::CFRelease(p) };

        assert!(cfstring("with\0embedded nul").is_none());
    }

    /// Toggling `inhibit_screensaver` twice with `false` should leave
    /// the held assertion at zero. We don't assert anything about the
    /// `true` case because that requires real IOKit privileges and
    /// would be flaky in CI.
    #[test]
    fn inhibit_screensaver_off_is_idempotent() {
        // First make sure nothing is held.
        let _ = inhibit_screensaver(false);
        assert_eq!(HELD_ASSERTION.load(Ordering::Acquire), 0);
        // Second call must still report success and not panic.
        assert!(inhibit_screensaver(false));
        assert_eq!(HELD_ASSERTION.load(Ordering::Acquire), 0);
    }

    /// `play_sound_async` with a bogus path must return `false` rather
    /// than panic. (It will still allocate an `NSSound` and `NSURL`,
    /// but AppKit refuses to initialise playback for a non-existent
    /// file and returns null.)
    #[test]
    fn play_sound_async_missing_file_returns_false() {
        assert!(!play_sound_async(Path::new(
            "/this/path/definitely/does/not/exist.aiff",
        )));
    }

    /// `get_program_path` must return a non-empty `PathBuf` on every
    /// macOS host. We don't assert the contents because test runners
    /// may exec into a workspace path we can't predict.
    #[test]
    fn get_program_path_nonempty() {
        let p = get_program_path();
        assert!(!p.as_os_str().is_empty());
    }
}