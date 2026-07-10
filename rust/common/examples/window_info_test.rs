//! End-to-end test of `rust/common/src/window_info.rs`.
//!
//! On **Windows** this opens the foreground window (the window the user
//! is currently looking at), reads its title with `GetWindowTextW`, its
//! bounding rectangle with `GetWindowRect`, and the DWM-computed
//! "extended frame" rectangle with `DwmGetWindowAttribute`
//! (`DWMWA_EXTENDED_FRAME_BOUNDS`). It then populates a `WindowInfo`
//! and calls [`WindowInfo::QueryRefreshRateForWindow`] to exercise the
//! dwmapi-backed DWM rate query.
//!
//! On **Linux** it tries to open the default X11 display. If no DISPLAY
//! is set (e.g. running under CI without an X server), it prints
//! "skipped: no X server" and exits 0. When a display is available it
//! opens the default root window, calls `XGetWindowAttributes` to read
//! the screen dimensions, populates a `WindowInfo`, and calls
//! [`WindowInfo::QueryRefreshRateForWindow`] to exercise the XRandR
//! rate query.
//!
//! Run with: `cargo run --release --example window_info_test`
//!
//! The example is intentionally a standalone binary — the module is
//! pulled in via `#[path = "../src/window_info.rs"]`, matching the
//! pattern used by the other `*_test` examples in this directory.

#[path = "../src/window_info.rs"]
mod window_info;
use window_info::{WindowInfo, WindowType};

fn main() {
    println!("== pcsx2 window_info end-to-end test ==");
    println!("platform: {}", std::env::consts::OS);

    #[cfg(target_os = "windows")]
    {
        run_windows();
    }

    #[cfg(target_os = "linux")]
    {
        run_linux();
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        println!("no native window-info backend on this platform — exiting 0");
    }

    println!("== done ==");
}

// ---------------------------------------------------------------------------
// Windows path: foreground window via user32 + dwmapi
// ---------------------------------------------------------------------------

#[cfg(target_os = "windows")]
fn run_windows() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::Graphics::Dwm::{
        DwmGetWindowAttribute, DWMWA_EXTENDED_FRAME_BOUNDS,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowRect, GetWindowTextW,
    };

    // --- 1. Query the foreground window ---------------------------------
    let hwnd: HWND = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        println!("Windows path: no foreground window available — exiting 0");
        return;
    }
    let hwnd_addr = hwnd as usize;
    println!("Windows path: GetForegroundWindow() = {:#x}", hwnd_addr);

    // --- 2. Title via GetWindowTextW ------------------------------------
    //
    // The title buffer is pre-sized at 512 wchar_t (1024 bytes) which is
    // enough for almost any window title. The C++ side uses the same
    // heuristic.
    let mut title_buf = [0u16; 512];
    let title_len = unsafe {
        GetWindowTextW(hwnd, title_buf.as_mut_ptr(), title_buf.len() as i32)
    };
    let title = if title_len > 0 {
        OsString::from_wide(&title_buf[..title_len as usize])
            .to_string_lossy()
            .into_owned()
    } else {
        String::from("<no title>")
    };
    println!("Windows path: window title = {:?}", title);

    // --- 3. Window rectangle via GetWindowRect --------------------------
    let mut rect: RECT = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetWindowRect(hwnd, &mut rect) };
    if ok != 0 {
        println!(
            "Windows path: GetWindowRect left={} top={} right={} bottom={} ({}x{})",
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            rect.right - rect.left,
            rect.bottom - rect.top
        );
    } else {
        println!("Windows path: GetWindowRect failed");
    }

    // --- 4. DWM extended-frame bounds via dwmapi ------------------------
    let mut dwm_rect: RECT = unsafe { std::mem::zeroed() };
    let hr = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS as u32,
            &mut dwm_rect as *mut _ as *mut _,
            std::mem::size_of::<RECT>() as u32,
        )
    };
    // HRESULT = i32. Success is hr >= 0.
    if hr >= 0 {
        println!(
            "Windows path: DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS) left={} top={} right={} bottom={} ({}x{})",
            dwm_rect.left,
            dwm_rect.top,
            dwm_rect.right,
            dwm_rect.bottom,
            dwm_rect.right - dwm_rect.left,
            dwm_rect.bottom - dwm_rect.top
        );
    } else {
        println!(
            "Windows path: DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS) returned HRESULT {:#x}",
            hr as u32
        );
    }

    // --- 5. Populate WindowInfo and exercise QueryRefreshRateForWindow --
    let mut info = WindowInfo::default();
    info.ty = WindowType::Win32;
    info.window_handle = hwnd as *mut _;
    info.surface_width = (rect.right - rect.left).max(0) as u32;
    info.surface_height = (rect.bottom - rect.top).max(0) as u32;
    match info.QueryRefreshRateForWindow() {
        Some(hz) => println!("Windows path: QueryRefreshRateForWindow = {:.3} Hz", hz),
        None => println!("Windows path: QueryRefreshRateForWindow = <none>"),
    }
}

// ---------------------------------------------------------------------------
// Linux path: XOpenDisplay + XGetWindowAttributes + XRandR
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn run_linux() {
    use std::ffi::CStr;

    // Open the default display. This is the gate the C++ side uses too;
    // if no DISPLAY is set XOpenDisplay returns null and the test prints
    // "skipped: no X server" and exits 0 — the example is still a clean
    // success on a headless box.
    let display = unsafe { x11::xlib::XOpenDisplay(std::ptr::null()) };
    if display.is_null() {
        println!("skipped: no X server (XOpenDisplay returned null)");
        return;
    }
    let display_name = unsafe { x11::xlib::XDisplayName(std::ptr::null()) };
    let display_name = if display_name.is_null() {
        String::from("<default>")
    } else {
        unsafe { CStr::from_ptr(display_name) }
            .to_string_lossy()
            .into_owned()
    };
    println!("Linux path: XOpenDisplay({:?}) = {:p}", display_name, display);

    // Open the default root window.
    let root = unsafe { x11::xlib::XDefaultRootWindow(display) };
    if root == 0 {
        println!("Linux path: XDefaultRootWindow returned 0");
        unsafe { x11::xlib::XCloseDisplay(display) };
        return;
    }
    println!("Linux path: XDefaultRootWindow = {:#x}", root);

    // Read window attributes for the root window — gives us the screen
    // dimensions in `width` / `height`.
    let mut attrs: x11::xlib::XWindowAttributes = unsafe { std::mem::zeroed() };
    let status = unsafe { x11::xlib::XGetWindowAttributes(display, root, &mut attrs) };
    if status != 0 {
        println!(
            "Linux path: XGetWindowAttributes(root) x={} y={} width={} height={} depth={} screen={}",
            attrs.x, attrs.y, attrs.width, attrs.height, attrs.depth, attrs.screen
        );
    } else {
        println!("Linux path: XGetWindowAttributes(root) failed");
    }

    // Populate WindowInfo and exercise QueryRefreshRateForWindow. The
    // C++ side passes `wi.display_connection = Display*` and
    // `wi.window_handle = (void*)Window`; we do the same.
    let mut info = WindowInfo::default();
    info.ty = WindowType::X11;
    info.display_connection = display as *mut _;
    info.window_handle = root as *mut _;
    info.surface_width = attrs.width as u32;
    info.surface_height = attrs.height as u32;
    match info.QueryRefreshRateForWindow() {
        Some(hz) => println!("Linux path: QueryRefreshRateForWindow = {:.3} Hz", hz),
        None => println!("Linux path: QueryRefreshRateForWindow = <none>"),
    }

    unsafe { x11::xlib::XCloseDisplay(display) };
    println!("Linux path: XCloseDisplay done");
}
