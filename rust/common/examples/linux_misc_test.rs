// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! End-to-end test of the Linux desktop-integration wrappers in
//! `linux_misc.rs`:
//!
//! - `inhibit_screensaver(true)` then `inhibit_screensaver(false)` over
//!   D-Bus (`org.freedesktop.ScreenSaver.Inhibit` / `UnInhibit`).
//! - `set_mouse_position(100, 100)` via X11 (`XWarpPointer`).
//!
//! Each test is wrapped in a `match` so a missing backend (no D-Bus
//! session on a CI box, no `$DISPLAY` on a headless server, or — in our
//! case — running on Windows) prints a clear "skipped" message rather
//! than failing. The point is to *prove the API compiles and links* on
//! the host platform; only on a real Linux desktop will the calls
//! actually move the mouse and inhibit the screensaver.
//!
//! Run with: cargo run --release --example linux_misc_test

#[cfg(target_os = "linux")]
mod linux_only {
    use pcsx2_common_rs::linux_misc::{inhibit_screensaver, set_mouse_position};

    /// Returns a `Some(())` only on a system with a usable D-Bus session.
    fn try_dbus() -> Option<()> {
        match dbus::blocking::Connection::new_session() {
            Ok(_) => Some(()),
            Err(e) => {
                println!("skipped: no D-Bus session ({})", e);
                None
            }
        }
    }

    /// Returns a `Some(())` only on a system with a reachable X server.
    fn try_x11() -> Option<()> {
        use x11::xlib::XOpenDisplay;
        unsafe {
            let dpy = XOpenDisplay(std::ptr::null());
            if dpy.is_null() {
                println!("skipped: no X server (DISPLAY not set or unreachable)");
                None
            } else {
                use x11::xlib::XCloseDisplay;
                XCloseDisplay(dpy);
                Some(())
            }
        }
    }

    pub fn run() {
        println!("=== screensaver inhibit / uninhibit (D-Bus) ===");
        if try_dbus().is_some() {
            let ok_inhibit = inhibit_screensaver(true);
            println!("inhibit_screensaver(true)  -> {}", ok_inhibit);
            // A second inhibit call must return false (duplicate guard).
            let ok_inhibit_again = inhibit_screensaver(true);
            println!("inhibit_screensaver(true)  -> {} (should be false on a real desktop)",
                     ok_inhibit_again);
            let ok_uninhibit = inhibit_screensaver(false);
            println!("inhibit_screensaver(false) -> {}", ok_uninhibit);
        } else {
            println!("no D-Bus session bus available — skipping screensaver test");
        }
        println!();

        println!("=== mouse warp (X11 / XWarpPointer) ===");
        if try_x11().is_some() {
            // Set the position. There is no portable way to read the
            // current position back without also opening the display, so
            // we just call and report that the call completed without
            // panicking. On a real desktop the cursor physically jumps.
            set_mouse_position(100, 100);
            println!("set_mouse_position(100, 100) called (cursor should have moved)");
        } else {
            println!("no X server available — skipping mouse warp test");
        }
        println!();

        println!("OK — linux_misc API exercised end-to-end (skipped steps printed above).");
    }
}

fn main() {
    println!("linux_misc end-to-end test");
    println!("==========================");
    println!("platform: {}", std::env::consts::OS);
    println!();

    #[cfg(target_os = "linux")]
    {
        linux_only::run();
    }

    #[cfg(not(target_os = "linux"))]
    {
        println!("skipped: not Linux (this platform is {}).", std::env::consts::OS);
        println!("Re-run on a Linux desktop to actually inhibit the screensaver and warp the mouse.");
        println!();
        println!("OK — example compiled and ran on a non-Linux host.");
    }
}
