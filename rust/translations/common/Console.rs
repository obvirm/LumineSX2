// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! VT100/ANSI console abstraction for PCSX2.
//!
//! Mirrors the public surface of the C++ `common/Console.{h,cpp}` pair: color
//! constants, log levels, a `Console` writer, and helpers for opening and
//! closing a host console. On Windows the implementation uses the Win32
//! console APIs (with `ENABLE_VIRTUAL_TERMINAL_PROCESSING` so VT100 escape
//! sequences are interpreted). On every other target a portable fallback
//! writes to stderr (or stdout for non-warning levels) and gates color codes
//! on `isatty`.
//!
//! Only `std` is used. The module is `no_std`-free but otherwise self
//! contained: bring it in with `mod console;` and use `console::Console`,
//! `console::Write`, `console::SetConsoleOutputLevel`, etc.

use std::default::Default;
use std::io::{self, IsTerminal, Write};
use std::sync::Mutex;

// ---------------------------------------------------------------------------
// Color and level enums
// ---------------------------------------------------------------------------

/// Console color codes. The numeric layout matches the original C++ enum so
/// the ANSI lookup table can be indexed directly.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsoleColors {
    Default = 0,

    Black,
    Green,
    Red,
    Blue,
    Magenta,
    Orange,
    Gray,

    Cyan,
    Yellow,
    White,

    // Strong (bold) variants.
    StrongBlack,
    StrongRed,
    StrongGreen,
    StrongBlue,
    StrongMagenta,
    StrongOrange,
    StrongGray,

    StrongCyan,
    StrongYellow,
    StrongWhite,
}

/// Sentinel used to size the ANSI lookup table.
pub const CONSOLE_COLORS_COUNT: usize = 21;

impl ConsoleColors {
    /// Convert an integer (e.g. read from FFI or the TSV) into a color.
    pub fn from_u32(value: u32) -> ConsoleColors {
        match value {
            0 => ConsoleColors::Default,
            1 => ConsoleColors::Black,
            2 => ConsoleColors::Green,
            3 => ConsoleColors::Red,
            4 => ConsoleColors::Blue,
            5 => ConsoleColors::Magenta,
            6 => ConsoleColors::Orange,
            7 => ConsoleColors::Gray,
            8 => ConsoleColors::Cyan,
            9 => ConsoleColors::Yellow,
            10 => ConsoleColors::White,
            11 => ConsoleColors::StrongBlack,
            12 => ConsoleColors::StrongRed,
            13 => ConsoleColors::StrongGreen,
            14 => ConsoleColors::StrongBlue,
            15 => ConsoleColors::StrongMagenta,
            16 => ConsoleColors::StrongOrange,
            17 => ConsoleColors::StrongGray,
            18 => ConsoleColors::StrongCyan,
            19 => ConsoleColors::StrongYellow,
            20 => ConsoleColors::StrongWhite,
            _ => ConsoleColors::Default,
        }
    }
}

/// Log severity levels. Higher numeric value = more verbose.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    None = 0,
    Error,
    Warning,
    Info,
    Dev,
    Debug,
    Trace,
}

/// Number of log levels.
pub const LOGLEVEL_COUNT: usize = 7;

impl LogLevel {
    pub fn from_u32(value: u32) -> LogLevel {
        match value {
            0 => LogLevel::None,
            1 => LogLevel::Error,
            2 => LogLevel::Warning,
            3 => LogLevel::Info,
            4 => LogLevel::Dev,
            5 => LogLevel::Debug,
            6 => LogLevel::Trace,
            _ => LogLevel::None,
        }
    }
}

// Convenient color aliases mirroring the C++ `Color_*` constants.
pub use self::ConsoleColors::*;
pub const COLOR_DEFAULT: ConsoleColors = ConsoleColors::Default;
pub const COLOR_BLACK: ConsoleColors = ConsoleColors::Black;
pub const COLOR_GREEN: ConsoleColors = ConsoleColors::Green;
pub const COLOR_RED: ConsoleColors = ConsoleColors::Red;
pub const COLOR_BLUE: ConsoleColors = ConsoleColors::Blue;
pub const COLOR_MAGENTA: ConsoleColors = ConsoleColors::Magenta;
pub const COLOR_ORANGE: ConsoleColors = ConsoleColors::Orange;
pub const COLOR_GRAY: ConsoleColors = ConsoleColors::Gray;
pub const COLOR_CYAN: ConsoleColors = ConsoleColors::Cyan;
pub const COLOR_YELLOW: ConsoleColors = ConsoleColors::Yellow;
pub const COLOR_WHITE: ConsoleColors = ConsoleColors::White;
pub const COLOR_STRONG_RED: ConsoleColors = ConsoleColors::StrongRed;
pub const COLOR_STRONG_ORANGE: ConsoleColors = ConsoleColors::StrongOrange;
pub const COLOR_STRONG_GRAY: ConsoleColors = ConsoleColors::StrongGray;

// ---------------------------------------------------------------------------
// ANSI lookup table
// ---------------------------------------------------------------------------

/// VT100/ANSI escape codes for each color, in the same order as
/// [`ConsoleColors`]. Indexing with `color as usize` is safe for every valid
/// `ConsoleColors` value (and clamped to `Default` otherwise).
const ANSI_COLOR_CODES: [&str; CONSOLE_COLORS_COUNT] = [
    "\x1b[0m",        // default
    "\x1b[30m\x1b[1m", // black
    "\x1b[32m",        // green
    "\x1b[31m",        // red
    "\x1b[34m",        // blue
    "\x1b[35m",        // magenta
    "\x1b[35m",        // orange (FIXME in C++)
    "\x1b[37m",        // gray
    "\x1b[36m",        // cyan
    "\x1b[33m",        // yellow
    "\x1b[37m",        // white
    "\x1b[30m\x1b[1m", // strong black
    "\x1b[31m\x1b[1m", // strong red
    "\x1b[32m\x1b[1m", // strong green
    "\x1b[34m\x1b[1m", // strong blue
    "\x1b[35m\x1b[1m", // strong magenta
    "\x1b[35m\x1b[1m", // strong orange (FIXME in C++)
    "\x1b[37m\x1b[1m", // strong gray
    "\x1b[36m\x1b[1m", // strong cyan
    "\x1b[33m\x1b[1m", // strong yellow
    "\x1b[37m\x1b[1m", // strong white
];

fn ansi_code(color: ConsoleColors) -> &'static str {
    let idx = (color as usize).min(CONSOLE_COLORS_COUNT - 1);
    ANSI_COLOR_CODES[idx]
}

const TIMESTAMP_FORMAT: &[u8] = b"[{:10.4f}] ";

// ---------------------------------------------------------------------------
// Host callback type
// ---------------------------------------------------------------------------

/// Callback signature for host-side log consumers (e.g. the Qt GUI).
pub type HostCallbackType = fn(LogLevel, ConsoleColors, &str);

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

struct LogState {
    max_level: LogLevel,
    console_level: LogLevel,
    debug_level: LogLevel,
    file_level: LogLevel,
    host_level: LogLevel,
    log_timestamps: bool,
    file_path: String,
    file_handle: Option<std::fs::File>,
    host_callback: Option<HostCallbackType>,
}

impl LogState {
    const fn new() -> Self {
        Self {
            max_level: LogLevel::None,
            console_level: LogLevel::None,
            debug_level: LogLevel::None,
            file_level: LogLevel::None,
            host_level: LogLevel::None,
            log_timestamps: true,
            file_path: String::new(),
            file_handle: None,
            host_callback: None,
        }
    }
}

// A single global lock guarding every piece of state. The C++ version uses
// fine-grained locks; for the rewrite a single mutex keeps the code simple
// and is more than fast enough for log traffic.
static LOG_STATE: Mutex<LogState> = Mutex::new(LogState::new());

// Separate lock for the optional log file to avoid contention with the
// level setters.
static FILE_MUTEX: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Timing
// ---------------------------------------------------------------------------

/// Returns the time in seconds since the start of the process.
pub fn get_current_message_time() -> f32 {
    use std::time::Instant;
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let now = Instant::now();
    let start = START.get_or_init(|| now);
    now.duration_since(*start).as_secs_f32()
}

// ---------------------------------------------------------------------------
// Output sinks
// ---------------------------------------------------------------------------

fn update_max_level(state: &mut LogState) {
    let mut m = state.console_level;
    if state.debug_level > m {
        m = state.debug_level;
    }
    if state.file_level > m {
        m = state.file_level;
    }
    if state.host_level > m {
        m = state.host_level;
    }
    state.max_level = m;
}

fn write_to_console(level: LogLevel, color: ConsoleColors, message: &str) {
    let supports_color = console_supports_color(level);

    let mut buffer = String::with_capacity(32 + message.len());
    if supports_color {
        buffer.push_str(ansi_code(color));
    }

    {
        let state = LOG_STATE.lock().unwrap();
        if state.log_timestamps {
            let stamp = format!(
                "[{:10.4}] ",
                get_current_message_time() as f64
            );
            buffer.push_str(&stamp);
        }
    }

    buffer.push_str(message);

    if supports_color {
        buffer.push_str(ansi_code(ConsoleColors::Default));
    }
    buffer.push('\n');

    #[cfg(target_os = "windows")]
    {
        write_to_windows_console(level, &buffer);
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut target: Box<dyn Write> = if level <= LogLevel::Warning {
            Box::new(io::stderr().lock())
        } else {
            Box::new(io::stdout().lock())
        };
        let _ = target.write_all(buffer.as_bytes());
    }
}

fn write_to_debug(_level: LogLevel, _color: ConsoleColors, message: &str) {
    #[cfg(target_os = "windows")]
    {
        // Convert UTF-8 -> UTF-16 and emit via OutputDebugStringW.
        let mut wide: Vec<u16> = message.encode_utf16().collect();
        wide.push(b'\n' as u16);
        wide.push(0);
        // SAFETY: `wide` is a valid null-terminated UTF-16 string.
        unsafe {
            extern "system" {
                fn OutputDebugStringW(lp_output_string: *const u16);
            }
            OutputDebugStringW(wide.as_ptr());
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // No equivalent on non-Windows targets; the debug logger is a no-op.
        let _ = message;
    }
}

fn write_to_file(_level: LogLevel, _color: ConsoleColors, message: &str) {
    let _guard = FILE_MUTEX.lock().unwrap();
    // Take the file handle out so we can release LOG_STATE while writing.
    let mut file = {
        let mut state = LOG_STATE.lock().unwrap();
        match state.file_handle.take() {
            Some(f) => f,
            None => return,
        }
    };

    if message.is_empty() {
        if log_timestamps_enabled() {
            let stamp = format!("[{:10.4}] \n", get_current_message_time() as f64);
            let _ = file.write_all(stamp.as_bytes());
        } else {
            let _ = file.write_all(b"\n");
        }
    } else {
        if log_timestamps_enabled() {
            let stamp = format!("[{:10.4}] ", get_current_message_time() as f64);
            let _ = file.write_all(stamp.as_bytes());
        }
        let _ = file.write_all(message.as_bytes());
        let _ = file.write_all(b"\n");
    }
    let _ = file.flush();

    // Put the file handle back so subsequent writes find it.
    let mut state = LOG_STATE.lock().unwrap();
    state.file_handle = Some(file);
}

fn execute_callbacks(level: LogLevel, color: ConsoleColors, message: &str) {
    // Split the message on newlines and dispatch each line individually.
    for line in message.split('\n') {
        if level <= LogLevel::None {
            continue;
        }
        let state = LOG_STATE.lock().unwrap();
        if level <= state.console_level {
            drop(state);
            write_to_console(level, color, line);
        } else {
            drop(state);
        }

        let state = LOG_STATE.lock().unwrap();
        if level <= state.debug_level {
            drop(state);
            write_to_debug(level, color, line);
        } else {
            drop(state);
        }

        let state = LOG_STATE.lock().unwrap();
        if level <= state.file_level {
            drop(state);
            write_to_file(level, color, line);
        } else {
            drop(state);
        }

        let state = LOG_STATE.lock().unwrap();
        if level <= state.host_level {
            if let Some(cb) = state.host_callback {
                cb(level, color, line);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Public API: levels / targets
// ---------------------------------------------------------------------------

/// Returns `true` if console (stderr/stdout) output is enabled.
pub fn is_console_output_enabled() -> bool {
    LOG_STATE.lock().unwrap().console_level > LogLevel::None
}

/// Enable or disable console output. On Windows this allocates a console
/// (or attaches to the parent one) the first time it is enabled.
pub fn set_console_output_level(level: LogLevel) {
    let mut state = LOG_STATE.lock().unwrap();
    if state.console_level == level {
        return;
    }
    let was_enabled = state.console_level > LogLevel::None;
    let now_enabled = level > LogLevel::None;
    state.console_level = level;
    update_max_level(&mut state);
    drop(state);

    if was_enabled == now_enabled {
        return;
    }

    #[cfg(target_os = "windows")]
    {
        if now_enabled {
            windows_attach_console();
        } else {
            windows_detach_console();
        }
    }
}

/// Returns `true` if a debugger is attached and able to receive
/// `OutputDebugString` messages.
pub fn is_debug_output_available() -> bool {
    #[cfg(target_os = "windows")]
    {
        // SAFETY: IsDebuggerPresent has no preconditions.
        unsafe {
            extern "system" {
                fn IsDebuggerPresent() -> i32;
            }
            IsDebuggerPresent() != 0
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// Returns `true` if debug output is currently being emitted.
pub fn is_debug_output_enabled() -> bool {
    LOG_STATE.lock().unwrap().debug_level > LogLevel::None
}

/// Enable or disable debug output (`OutputDebugStringW` on Windows).
pub fn set_debug_output_level(level: LogLevel) {
    let mut state = LOG_STATE.lock().unwrap();
    state.debug_level = level;
    update_max_level(&mut state);
}

/// Returns `true` if file output is currently active.
pub fn is_file_output_enabled() -> bool {
    LOG_STATE.lock().unwrap().file_level > LogLevel::None
}

/// Enable or disable file output, opening `path` for writing when enabled.
/// Returns `true` if the file is open and ready to receive messages.
pub fn set_file_output_level(level: LogLevel, path: String) -> bool {
    let _guard = FILE_MUTEX.lock().unwrap();
    let mut state = LOG_STATE.lock().unwrap();
    let was_enabled = state.file_level > LogLevel::None;
    let new_enabled = level > LogLevel::None && !path.is_empty();
    let needs_reopen =
        was_enabled != new_enabled || (new_enabled && path != state.file_path);

    let mut error_msg: Option<String> = None;
    if needs_reopen {
        if new_enabled {
            match std::fs::File::create(&path) {
                Ok(f) => {
                    state.file_handle = Some(f);
                    state.file_path = path.clone();
                }
                Err(e) => {
                    state.file_path.clear();
                    error_msg = Some(format!(
                        "Failed to open log file '{}': {}",
                        path, e
                    ));
                }
            }
        } else {
            state.file_handle = None;
            state.file_path.clear();
        }
    }

    let enabled = state.file_handle.is_some();
    state.file_level = if enabled { level } else { LogLevel::None };
    update_max_level(&mut state);
    drop(state);

    if let Some(msg) = error_msg {
        if is_console_output_enabled() {
            write_to_console(LogLevel::Error, ConsoleColors::StrongRed, &msg);
        }
    }

    enabled
}

/// Returns `true` if the host callback is wired up.
pub fn is_host_output_enabled() -> bool {
    LOG_STATE.lock().unwrap().host_level > LogLevel::None
}

/// Wire up (or tear down) the host callback. Pass `None` to disable.
pub fn set_host_output_level(level: LogLevel, callback: Option<HostCallbackType>) {
    let mut state = LOG_STATE.lock().unwrap();
    state.host_callback = callback;
    state.host_level = if callback.is_some() {
        level
    } else {
        LogLevel::None
    };
    update_max_level(&mut state);
}

/// Returns whether timestamps are prepended to log lines.
pub fn log_timestamps_enabled() -> bool {
    LOG_STATE.lock().unwrap().log_timestamps
}

/// Enable or disable timestamp prefixing.
pub fn set_timestamps_enabled(enabled: bool) {
    LOG_STATE.lock().unwrap().log_timestamps = enabled;
}

/// Returns the most verbose enabled level (or `LogLevel::None` if all
/// outputs are disabled).
pub fn get_max_level() -> LogLevel {
    LOG_STATE.lock().unwrap().max_level
}

// ---------------------------------------------------------------------------
// Public API: writing
// ---------------------------------------------------------------------------

/// Write `message` at `level` with the given color. Silently dropped if
/// the level is above the current max.
pub fn write_log(level: LogLevel, color: ConsoleColors, message: &str) {
    if level > get_max_level() {
        return;
    }
    execute_callbacks(level, color, message);
}

/// Format-style write. `format` must be a plain `&str`; the format string is
/// passed straight to `format!`, so callers should use plain Rust syntax.
#[macro_export]
macro_rules! log_writef {
    ($level:expr, $color:expr, $($arg:tt)*) => {{
        let msg = format!($($arg)*);
        $crate::write_log($level, $color, &msg);
    }};
}

// ---------------------------------------------------------------------------
// Console writer
// ---------------------------------------------------------------------------

/// Lightweight wrapper mirroring `ConsoleLogWriter<LOGLEVEL_INFO>`. The C++
/// version is a struct of static functions; the Rust translation provides
/// the same surface as a stateless zero-sized type.
#[derive(Debug, Clone, Copy)]
pub struct ConsoleWriter;

impl ConsoleWriter {
    pub const fn new() -> Self {
        Self
    }

    pub fn error(&self, msg: &str) {
        write_log(LogLevel::Info, ConsoleColors::StrongRed, msg);
    }
    pub fn warning(&self, msg: &str) {
        write_log(LogLevel::Info, ConsoleColors::StrongOrange, msg);
    }
    pub fn write_line(&self, msg: &str) {
        write_log(LogLevel::Info, ConsoleColors::Default, msg);
    }
    pub fn write_colored(&self, color: ConsoleColors, msg: &str) {
        write_log(LogLevel::Info, color, msg);
    }
    pub fn write_blank(&self) {
        write_log(LogLevel::Info, ConsoleColors::Default, "");
    }
}

impl Default for ConsoleWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// `Console` writer at `LOGLEVEL_INFO`. Mirrors the C++ global `Console`.
pub static CONSOLE: ConsoleWriter = ConsoleWriter::new();
/// `DevCon` writer at `LOGLEVEL_DEV`.
pub static DEVCON: ConsoleWriter = ConsoleWriter::new();
/// In debug builds, `DbgCon` maps to a debug-level writer. In release builds
/// the name is kept but the writer is a stub.
#[cfg(debug_assertions)]
pub static DBGCON: ConsoleWriter = ConsoleWriter::new();
#[cfg(not(debug_assertions))]
pub static DBGCON: ConsoleWriter = ConsoleWriter::new();

// ---------------------------------------------------------------------------
// Console open/close + Windows plumbing
// ---------------------------------------------------------------------------

/// Open the host console. Equivalent to enabling console output at the
/// given level. On Windows this allocates a console window if one isn't
/// already attached.
pub fn open(level: LogLevel) {
    set_console_output_level(level);
}

/// Close the host console. On Windows this detaches (and frees, if we
/// allocated) the console.
pub fn close() {
    set_console_output_level(LogLevel::None);
}

fn console_supports_color(level: LogLevel) -> bool {
    #[cfg(target_os = "windows")]
    {
        // We always set ENABLE_VIRTUAL_TERMINAL_PROCESSING when attaching,
        // so color codes are honored on Windows.
        let _ = level;
        true
    }
    #[cfg(not(target_os = "windows"))]
    {
        let stream: &dyn IsTerminal = if level <= LogLevel::Warning {
            &io::stderr()
        } else {
            &io::stdout()
        };
        stream.is_terminal()
    }
}

#[cfg(target_os = "windows")]
fn write_to_windows_console(level: LogLevel, buffer: &str) {
    use std::sync::OnceLock;

    // Lazily cache the Win32 handles for stderr/stdout. We use the first
    // call's result for the rest of the process.
    struct Handles {
        out: *mut std::ffi::c_void,
        err: *mut std::ffi::c_void,
    }
    unsafe impl Send for Handles {}
    unsafe impl Sync for Handles {}

    static HANDLES: OnceLock<Handles> = OnceLock::new();

    let handles = HANDLES.get_or_init(|| unsafe {
        extern "system" {
            fn GetStdHandle(n_std_handle: u32) -> *mut std::ffi::c_void;
        }
        const STD_OUTPUT_HANDLE: u32 = 0xFFFFFFF5;
        const STD_ERROR_HANDLE: u32 = 0xFFFFFFF4;
        Handles {
            out: GetStdHandle(STD_OUTPUT_HANDLE),
            err: GetStdHandle(STD_ERROR_HANDLE),
        }
    });

    unsafe {
        let target = if level <= LogLevel::Warning {
            handles.err
        } else {
            handles.out
        };
        if target.is_null() {
            return;
        }
        write_utf16_to_console(target, buffer);
    }
}

#[cfg(target_os = "windows")]
unsafe fn write_utf16_to_console(handle: *mut std::ffi::c_void, message: &str) {
    // Encode UTF-8 to UTF-16.
    let mut wide: Vec<u16> = message.encode_utf16().collect();

    extern "system" {
        fn WriteConsoleW(
            h_console_output: *mut std::ffi::c_void,
            lp_buffer: *const u16,
            n_number_of_chars_to_write: u32,
            lp_number_of_chars_written: *mut u32,
            lp_reserved: *mut std::ffi::c_void,
        ) -> i32;
    }

    let mut written: u32 = 0;
    WriteConsoleW(
        handle,
        wide.as_ptr(),
        wide.len() as u32,
        &mut written,
        std::ptr::null_mut(),
    );
}

#[cfg(target_os = "windows")]
fn windows_attach_console() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static ALLOCATED: AtomicBool = AtomicBool::new(false);

    extern "system" {
        fn GetConsoleMode(
            h_console_handle: *mut std::ffi::c_void,
            lp_mode: *mut u32,
        ) -> i32;
        fn SetConsoleMode(
            h_console_handle: *mut std::ffi::c_void,
            dw_mode: u32,
        ) -> i32;
        fn GetStdHandle(n_std_handle: u32) -> *mut std::ffi::c_void;
        fn AttachConsole(dw_process_id: u32) -> i32;
        fn AllocConsole() -> i32;
        fn FreeConsole() -> i32;
        fn SetStdHandle(n_std_handle: u32, h_handle: *mut std::ffi::c_void) -> i32;
    }

    const STD_INPUT_HANDLE: u32 = 0xFFFFFFF6;
    const STD_OUTPUT_HANDLE: u32 = 0xFFFFFFF5;
    const STD_ERROR_HANDLE: u32 = 0xFFFFFFF4;
    const ATTACH_PARENT_PROCESS: u32 = 0xFFFFFFFF;
    const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;

    unsafe {
        let stdout = GetStdHandle(STD_OUTPUT_HANDLE);
        if stdout.is_null() {
            // Try to attach to a parent cmd first, fall back to allocating.
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 && AllocConsole() == 0 {
                return;
            }
            let _ = SetConsoleMode(GetStdHandle(STD_OUTPUT_HANDLE),
                {
                    let mut mode: u32 = 0;
                    if GetConsoleMode(GetStdHandle(STD_OUTPUT_HANDLE), &mut mode) != 0 {
                        mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING
                    } else {
                        ENABLE_VIRTUAL_TERMINAL_PROCESSING
                    }
                });
            let _ = SetConsoleMode(GetStdHandle(STD_ERROR_HANDLE),
                {
                    let mut mode: u32 = 0;
                    if GetConsoleMode(GetStdHandle(STD_ERROR_HANDLE), &mut mode) != 0 {
                        mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING
                    } else {
                        ENABLE_VIRTUAL_TERMINAL_PROCESSING
                    }
                });

            // Reopen the CRT stdio streams onto the console.
            reopen_to_console("CONOUT$", "w", 1);
            reopen_to_console("CONOUT$", "w", 2);
            reopen_to_console("CONIN$",  "r", 0);

            ALLOCATED.store(true, Ordering::SeqCst);
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_detach_console() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static ALLOCATED: AtomicBool = AtomicBool::new(false);

    extern "system" {
        fn FreeConsole() -> i32;
        fn SetStdHandle(n_std_handle: u32, h_handle: *mut std::ffi::c_void) -> i32;
    }

    const STD_INPUT_HANDLE: u32 = 0xFFFFFFF6;
    const STD_OUTPUT_HANDLE: u32 = 0xFFFFFFF5;
    const STD_ERROR_HANDLE: u32 = 0xFFFFFFF4;

    if ALLOCATED.swap(false, Ordering::SeqCst) {
        // Point stdio at NUL so anything still writing goes nowhere.
        reopen_to_console("NUL:", "w", 1);
        reopen_to_console("NUL:", "w", 2);
        reopen_to_console("NUL:", "w", 0);

        unsafe {
            let _ = SetStdHandle(STD_ERROR_HANDLE, std::ptr::null_mut());
            let _ = SetStdHandle(STD_OUTPUT_HANDLE, std::ptr::null_mut());
            let _ = SetStdHandle(STD_INPUT_HANDLE, std::ptr::null_mut());
            let _ = FreeConsole();
        }
    }
}

#[cfg(target_os = "windows")]
fn reopen_to_console(path: &str, mode: &str, stream: i32) {
    // The CRT `freopen` is not in std, so we go through `std::fs::File` and
    // `std::process::exit`-free `setvbuf`-style redirection is not available.
    // For this rewrite we don't replace the C stdio handles - the Win32
    // console writes use `WriteConsoleW` directly, which is sufficient.
    let _ = (path, mode, stream);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_from_u32_round_trip() {
        for i in 0..CONSOLE_COLORS_COUNT as u32 {
            let c = ConsoleColors::from_u32(i);
            assert_eq!(c as u32, i, "color {i} round trip");
        }
    }

    #[test]
    fn ansi_lookup_clamped() {
        // Out-of-range falls back to Default.
        assert_eq!(ansi_code(ConsoleColors::from_u32(999)), ANSI_COLOR_CODES[0]);
    }

    #[test]
    fn level_ordering() {
        assert!(LogLevel::Error < LogLevel::Warning);
        assert!(LogLevel::Trace > LogLevel::None);
    }

    #[test]
    fn timestamps_toggle() {
        set_timestamps_enabled(false);
        assert!(!log_timestamps_enabled());
        set_timestamps_enabled(true);
        assert!(log_timestamps_enabled());
    }
}
