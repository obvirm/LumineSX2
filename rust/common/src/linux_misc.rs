// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Linux-specific host utilities.
//!
//! Pure-Rust reimplementation of `common/Linux/LnxMisc.cpp`. The relevant
//! pieces in scope here are:
//!
//! - Screensaver inhibition over D-Bus
//!   (`org.freedesktop.ScreenSaver` / `org.mate.ScreenSaver`).
//! - Asynchronous sound playback via a shell-out to `aplay` (with
//!   `gst-play-1.0` and `gst-launch-1.0` fallbacks).
//! - Mouse positioning and raw-motion tracking via X11.
//!
//! All other helpers from the original file (`GetPhysicalMemory`,
//! `GetTickFrequency`, `Threading::Sleep`, ...) belong to their respective
//! modules and live elsewhere in this crate.
//!
//! # Cargo.toml
//!
//! ```toml
//! [target.'cfg(target_os = "linux")'.dependencies]
//! dbus = "0.9"
//! x11 = "2"
//! once_cell = "1"
//! ```
//!
//! # Crate choice notes
//!
//! The `dbus` 0.9 crate provides a blocking-send wrapper
//! (`dbus::blocking::Connection`) that mirrors the original C++ call to
//! `dbus_connection_send_with_reply_and_block` exactly: it takes a
//! `Message` with arguments already appended, sends it, blocks, and
//! returns a `Reply`. We read the cookie out of the reply with `read1()`.
//!
//! The `x11` 2.x crate is a direct binding to the upstream Xlib / XInput
//! headers. There is no higher-level wrapper for `XWarpPointer` or
//! `XISelectEvents`, so the mouse-warp and motion-tracking paths use the
//! raw FFI functions via the `x11::xlib` and `x11::xinput2` modules. The
//! crate does give us proper type names (`Display`, `Window`, `XEvent`,
//! `XIEventMask`, ...) and the `link!()` macro that pulls in `libX11` and
//! `libXi` automatically.

#![cfg(target_os = "linux")]

use std::os::raw::c_char;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread::{self, JoinHandle};

use once_cell::sync::Lazy;

// ---------------------------------------------------------------------------
// Screensaver inhibition (D-Bus)
// ---------------------------------------------------------------------------

/// Cookie returned by `org.freedesktop.ScreenSaver.Inhibit`. Needed to call
/// `UnInhibit` later. Guarded by a `Mutex` so a second `Inhibit` request can
/// be rejected (matches the C++ duplicate-inhibitor guard).
static SCREENSAVER_COOKIE: Lazy<Mutex<Option<u32>>> = Lazy::new(|| Mutex::new(None));

/// Hardcoded D-Bus service identifiers for the freedesktop and MATE
/// ScreenSaver services.
const SCREENSAVER_DEST_FDO: &str = "org.freedesktop.ScreenSaver";
const SCREENSAVER_PATH_FDO: &str = "/org/freedesktop/ScreenSaver";
const SCREENSAVER_IFACE_FDO: &str = "org.freedesktop.ScreenSaver";

const SCREENSAVER_DEST_MATE: &str = "org.mate.ScreenSaver";
const SCREENSAVER_PATH_MATE: &str = "/org/mate/ScreenSaver";
const SCREENSAVER_IFACE_MATE: &str = "org.mate.ScreenSaver";

/// Inhibit (or un-inhibit) the desktop screensaver.
///
/// Returns `true` on success. Mirrors the C++ semantics exactly:
/// - If `inhibit` is `true` and an inhibitor is already active, the call
///   returns `false` (no duplicate cookie is issued).
/// - If `inhibit` is `false` and no cookie is held, the call still returns
///   `true` (the desktop will simply ignore the `UnInhibit`).
pub fn inhibit_screensaver(inhibit: bool) -> bool {
    // Pick the right service based on the desktop environment, matching the
    // original `std::strncmp(desktop_session, "mate", 4)` check.
    let desktop_is_mate = std::env::var("DESKTOP_SESSION")
        .map(|s| s.starts_with("mate"))
        .unwrap_or(false);
    let (dest, path_str, iface) = if desktop_is_mate {
        (
            SCREENSAVER_DEST_MATE,
            SCREENSAVER_PATH_MATE,
            SCREENSAVER_IFACE_MATE,
        )
    } else {
        (
            SCREENSAVER_DEST_FDO,
            SCREENSAVER_PATH_FDO,
            SCREENSAVER_IFACE_FDO,
        )
    };

    // The blocking connection wraps a single D-Bus session connection.
    // On a system without a D-Bus session bus (e.g. CI, container) this
    // returns an error and we bail — same as the C++ behaviour.
    let conn = match dbus::blocking::Connection::new_session() {
        Ok(c) => c,
        Err(_) => return false,
    };

    // `with_path` builds a `ConnPath` that captures the destination, path
    // and a default timeout. The resulting handle's `method_call` is the
    // typed entry point: it appends args, sends, blocks, and returns the
    // reply deserialized into the requested `R` type.
    let proxy = conn.with_path(dest, path_str, ::std::time::Duration::from_millis(5_000));

    if inhibit {
        // Guard against repeat inhibitions: bail out without allocating a
        // second cookie.
        {
            let guard = SCREENSAVER_COOKIE.lock().unwrap();
            if guard.is_some() {
                return false;
            }
        }

        let program_name = "PCSX2";
        let reason = "PCSX2 VM is running.";

        // `Inhibit` returns a single `u32` cookie.
        let reply: Result<u32, dbus::Error> =
            proxy.method_call(iface, "Inhibit", (program_name, reason));
        match reply {
            Ok(cookie) => {
                *SCREENSAVER_COOKIE.lock().unwrap() = Some(cookie);
                true
            }
            Err(_) => false,
        }
    } else {
        let cookie = match SCREENSAVER_COOKIE.lock().unwrap().take() {
            Some(c) => c,
            None => return true, // Nothing to uninhibit.
        };

        // `UnInhibit` takes a single u32 (the cookie) and returns nothing.
        let reply: Result<(), dbus::Error> =
            proxy.method_call(iface, "UnInhibit", (cookie,));
        reply.is_ok()
    }
}

// ---------------------------------------------------------------------------
// Asynchronous sound playback (aplay / gst-play / gst-launch)
// ---------------------------------------------------------------------------

/// Play a sound file asynchronously by shelling out.
///
/// Tries `aplay` first, then `gst-play-1.0`, then a `gst-launch-1.0`
/// pipeline built from the file extension. Returns `true` if any of the
/// three successfully spawned a child process; the caller does not wait.
pub fn play_sound_async(path: &Path) -> bool {
    // Primary path: aplay.
    if Command::new("aplay")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
    {
        return true;
    }

    // Fallback: gst-play-1.0.
    if Command::new("gst-play-1.0")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
    {
        return true;
    }

    // Last-resort: gst-launch-1.0 with an extension-derived demuxer.
    let path_str = match path.to_str() {
        Some(s) => s,
        None => return false,
    };
    let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let location_arg = format!("location={}", path_str);
    let parse_arg = format!("{}parse", extension);

    if Command::new("gst-launch-1.0")
        .args(["filesrc", &location_arg, "!", &parse_arg, "!", "alsasink"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
    {
        return true;
    }

    eprintln!(
        "Failed to play sound effect {}. Make sure you have aplay, \
         gst-play-1.0, or gst-launch-1.0 available.",
        path.display()
    );
    false
}

// ---------------------------------------------------------------------------
// Mouse positioning (X11, one-shot warp)
// ---------------------------------------------------------------------------

use x11::xlib::{
    XCloseDisplay, XDefaultRootWindow, XFlush, XOpenDisplay, XWarpPointer,
};

/// Warp the X11 cursor to `(x, y)` in root-window coordinates.
///
/// Opens a transient `Display`, calls `XWarpPointer`, flushes, and closes —
/// identical to the C++ implementation. A no-op if X11 is unavailable.
pub fn set_mouse_position(x: i32, y: i32) {
    unsafe {
        let display = XOpenDisplay(std::ptr::null());
        if display.is_null() {
            return;
        }

        let root = XDefaultRootWindow(display);
        XWarpPointer(display, 0, root, 0, 0, 0, 0, x, y);
        XFlush(display);
        XCloseDisplay(display);
    }
}

// ---------------------------------------------------------------------------
// Mouse-position callback (X11, raw motion thread)
// ---------------------------------------------------------------------------

type MouseCb = Box<dyn Fn(i32, i32) + Send + 'static>;

/// Process-wide storage for the user's mouse-motion callback. Mutex-protected
/// so detach can clear it without racing the worker thread.
static MOUSE_CB: Lazy<Mutex<Option<MouseCb>>> = Lazy::new(|| Mutex::new(None));

/// Flag the worker thread polls to know when to stop.
static TRACKING_MOUSE: AtomicBool = AtomicBool::new(false);

/// Handle to the worker thread, so detach can join it cleanly.
static MOUSE_THREAD: Lazy<Mutex<Option<JoinHandle<()>>>> =
    Lazy::new(|| Mutex::new(None));

/// Register `cb` to be invoked with the latest `(root_x, root_y)` whenever the
/// mouse moves under the root window. Spawns a dedicated X11 thread the first
/// time it's called; subsequent calls just swap the callback.
pub fn attach_mouse_position_cb(cb: Box<dyn Fn(i32, i32) + Send + 'static>) {
    *MOUSE_CB.lock().unwrap() = Some(cb);

    if TRACKING_MOUSE.swap(true, Ordering::SeqCst) {
        // Thread already running.
        return;
    }

    let handle = thread::Builder::new()
        .name("X11 Mouse Thread".into())
        .spawn(mouse_event_loop)
        .expect("failed to spawn X11 mouse thread");

    *MOUSE_THREAD.lock().unwrap() = Some(handle);
}

/// Stop tracking mouse motion and clear the registered callback.
pub fn detach_mouse_position_cb() {
    TRACKING_MOUSE.store(false, Ordering::SeqCst);
    *MOUSE_CB.lock().unwrap() = None;

    if let Some(handle) = MOUSE_THREAD.lock().unwrap().take() {
        let _ = handle.join();
    }
}

/// Worker thread body — mirrors `mouseEventLoop` in the C++ source.
///
/// Uses the `x11` crate's `xinput2` bindings to subscribe to raw motion
/// events on the root window. The `x11` crate does not yet have a safe
/// wrapper for `XISelectEvents` / raw motion, so the body is `unsafe`.
fn mouse_event_loop() {
    use x11::xinput2::{
        XIAllDevices, XIEventMask, XIRawEvent, XISelectEvents, XISetMask, XI_RawMotion,
    };
    use x11::xlib::{
        XCloseDisplay, XDefaultRootWindow, XFreeEventData, XGetEventData, XNextEvent,
        XOpenDisplay, XPending, XQueryExtension, XQueryPointer, XSync, GenericEvent, XEvent,
    };

    unsafe {
        let display = XOpenDisplay(std::ptr::null());
        if display.is_null() {
            return;
        }

        // Verify the XInput extension is present.
        let mut opcode: i32 = 0;
        let mut eventcode: i32 = 0;
        let mut error: i32 = 0;
        if XQueryExtension(
            display,
            b"XInputExtension\0".as_ptr() as *const c_char,
            &mut opcode,
            &mut eventcode,
            &mut error,
        ) == 0
        {
            XCloseDisplay(display);
            return;
        }

        let root = XDefaultRootWindow(display);

        // Build the event mask byte buffer. `(XI_LASTEVENT + 7) / 8` mirrors
        // the C++ `unsigned char mask[(XI_LASTEVENT + 7) / 8]` static array.
        let mask_len = (x11::xinput2::XI_LASTEVENT as usize + 7) / 8;
        let mut mask = vec![0u8; mask_len];

        let mut evmask: XIEventMask = std::mem::zeroed();
        evmask.deviceid = XIAllDevices;
        evmask.mask_len = mask.len() as i32;
        evmask.mask = mask.as_mut_ptr();

        XISetMask(mask.as_mut_ptr(), XI_RawMotion);
        XISelectEvents(display, root, &mut evmask, 1);
        XSync(display, 0);

        while TRACKING_MOUSE.load(Ordering::SeqCst) {
            // `XPending` is non-blocking — if nothing is queued we yield and
            // re-check the stop flag. Same pattern as the C++ loop.
            if XPending(display) == 0 {
                std::thread::sleep(std::time::Duration::from_millis(1));
                std::hint::spin_loop();
                continue;
            }

            let mut event: XEvent = std::mem::zeroed();
            XNextEvent(display, &mut event);

            let cookie = event.xcookie;
            // Note: `cookie.type` is a reserved keyword in Rust, so we
            // access the field via struct field syntax instead.
            let cookie_type = cookie.type_;
            if cookie_type == GenericEvent
                && cookie.extension == opcode
                && XGetEventData(display, &mut event) != 0
            {
                let raw: *mut XIRawEvent = event.xcookie.data as *mut XIRawEvent;
                if (*raw).evtype == XI_RawMotion {
                    // Ask X11 where the pointer actually is right now. Same
                    // call signature as the C++ XQueryPointer.
                    let mut w: u64 = 0;
                    let mut root_x: i32 = 0;
                    let mut root_y: i32 = 0;
                    let mut win_x: i32 = 0;
                    let mut win_y: i32 = 0;
                    let mut mask_ret: u32 = 0;
                    XQueryPointer(
                        display,
                        root,
                        &mut w,
                        &mut w,
                        &mut root_x,
                        &mut root_y,
                        &mut win_x,
                        &mut win_y,
                        &mut mask_ret,
                    );

                    if let Some(cb) = MOUSE_CB.lock().unwrap().as_ref() {
                        cb(root_x, root_y);
                    }
                }
                XFreeEventData(display, &mut event);
            }
        }

        XCloseDisplay(display);
    }
}

// ---------------------------------------------------------------------------
// FFI surface
// ---------------------------------------------------------------------------

/// FFI: inhibit/uninhibit the screensaver.
#[no_mangle]
pub extern "C" fn pcsx2_linux_inhibit_screensaver(inhibit: bool) -> bool {
    inhibit_screensaver(inhibit)
}

/// FFI: play a sound asynchronously. The `path` must be a NUL-terminated UTF-8
/// C string. Returns `true` if a backend was successfully spawned.
#[no_mangle]
pub extern "C" fn pcsx2_linux_play_sound_async(path: *const c_char) -> bool {
    if path.is_null() {
        return false;
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(path) };
    let path = match cstr.to_str() {
        Ok(s) => std::path::Path::new(s),
        Err(_) => return false,
    };
    play_sound_async(path)
}

/// FFI: warp the mouse cursor.
#[no_mangle]
pub extern "C" fn pcsx2_linux_set_mouse_position(x: i32, y: i32) {
    set_mouse_position(x, y);
}
