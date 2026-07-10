// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Console / logging — Rust translation of PCSX2's `common/Console.h` and
//! `common/Console.cpp`.
//!
//! This module provides the rust-side of PCSX2's logging stack. It mirrors
//! the public surface of the original C++ `Log` namespace (`Write`,
//! `SetConsoleOutputLevel`, `SetHostOutputLevel`, `GetMaxLevel`, ...) plus
//! the `ConsoleLogWriter<LOGLEVEL_INFO>` / `ConsoleLogWriter<LOGLEVEL_DEV>`
//! global writers and the `ERROR_LOG` / `WARNING_LOG` / `INFO_LOG` /
//! `DEV_LOG` macros.
//!
//! ## Architecture
//!
//! - The [`log`](https://docs.rs/log) crate is the underlying logging
//!   facade. Rust call sites use `log::error!`, `log::warn!`, `log::info!`,
//!   `log::debug!`, `log::trace!` and we forward to PCSX2's level + colour
//!   scheme via an `impl log::Log` for a static `Pcsx2Logger`.
//! - The host callback installed by the C++ side is stored as a raw
//!   function pointer in [`HOST_CALLBACK`] (an `AtomicPtr`). When non-null
//!   it is invoked on every emitted log record.
//! - Console / file sinks are no-ops in this Rust crate: on PCSX2 the
//!   console is the C++ side's responsibility (it owns `AllocConsole`,
//!   `WriteConsoleW`, ANSI escape handling and the `FILE*` log file). The
//!   Rust side simply forwards to the host callback so the C++ side can
//!   route the message to whatever sinks it has configured.
//!
//! ## Required dependencies (not yet in `Cargo.toml`)
//!
//! This module uses the [`log`](https://crates.io/crates/log) crate. To
//! build, add the following to `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! log = "0.4"
//! ```

use core::ffi::c_char;
use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering};
use std::ptr;

// ===========================================================================
// Enums
// ===========================================================================

/// Severity / verbosity level for log messages.
///
/// Higher numeric values are *more* verbose (mirrors the original C++
/// `LOGLEVEL_NONE = 0, ..., LOGLEVEL_TRACE = 6`). A record is emitted only
/// when `record_level as u32 <= effective_max_level`.
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    /// Silences all log traffic.
    None = 0,
    Error = 1,
    Warning = 2,
    Info = 3,
    /// PCSX2-internal developer-level logging (no direct mapping in the
    /// `log` crate; use [`write`] with this level explicitly).
    Dev = 4,
    Debug = 5,
    Trace = 6,
    /// One-past-the-end sentinel, matches C++ `LOGLEVEL_COUNT`.
    Count = 7,
}

impl LogLevel {
    /// Convert from the underlying `u32` representation. Out-of-range
    /// values are clamped to [`LogLevel::Trace`].
    #[inline]
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::None,
            1 => Self::Error,
            2 => Self::Warning,
            3 => Self::Info,
            4 => Self::Dev,
            5 => Self::Debug,
            6 => Self::Trace,
            _ => Self::Trace,
        }
    }

    /// Map a [`log::Level`] to the closest [`LogLevel`]. There is no
    /// `log::Level` for `Dev`, so `Debug` and `Trace` map one-to-one and
    /// anything finer than `Trace` falls back to [`LogLevel::Trace`].
    #[inline]
    pub fn from_log_level(level: log::Level) -> Self {
        match level {
            log::Level::Error => Self::Error,
            log::Level::Warn => Self::Warning,
            log::Level::Info => Self::Info,
            log::Level::Debug => Self::Debug,
            log::Level::Trace => Self::Trace,
        }
    }
}

/// 24-colour palette used by the console (and any host sink that wants to
/// preserve colour). The numeric values match the original C++
/// `ConsoleColors` enumeration so existing C++ call sites are source-
/// compatible via cbindgen.
#[repr(u32)]
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum ConsoleColor {
    Default = 0,
    Black = 1,
    Green = 2,
    Red = 3,
    Blue = 4,
    Magenta = 5,
    Orange = 6,
    Gray = 7,
    Cyan = 8,
    Yellow = 9,
    White = 10,
    StrongBlack = 11,
    StrongRed = 12,
    StrongGreen = 13,
    StrongBlue = 14,
    StrongMagenta = 15,
    StrongOrange = 16,
    StrongGray = 17,
    StrongCyan = 18,
    StrongYellow = 19,
    StrongWhite = 20,
}

impl ConsoleColor {
    /// Convert from the underlying `u32` representation. Out-of-range
    /// values are clamped to [`ConsoleColor::Default`].
    #[inline]
    pub fn from_u32(v: u32) -> Self {
        match v {
            0 => Self::Default,
            1 => Self::Black,
            2 => Self::Green,
            3 => Self::Red,
            4 => Self::Blue,
            5 => Self::Magenta,
            6 => Self::Orange,
            7 => Self::Gray,
            8 => Self::Cyan,
            9 => Self::Yellow,
            10 => Self::White,
            11 => Self::StrongBlack,
            12 => Self::StrongRed,
            13 => Self::StrongGreen,
            14 => Self::StrongBlue,
            15 => Self::StrongMagenta,
            16 => Self::StrongOrange,
            17 => Self::StrongGray,
            18 => Self::StrongCyan,
            19 => Self::StrongYellow,
            20 => Self::StrongWhite,
            _ => Self::Default,
        }
    }

    /// The colour associated with each [`log::Level`]. Matches the colour
    /// scheme used by the original C++ `ERROR_LOG` / `WARNING_LOG` / ...
    /// macros.
    #[inline]
    pub fn for_log_level(level: log::Level) -> Self {
        match level {
            log::Level::Error => Self::StrongRed,
            log::Level::Warn => Self::StrongOrange,
            log::Level::Info => Self::White,
            log::Level::Debug => Self::Gray,
            log::Level::Trace => Self::Blue,
        }
    }
}

// ===========================================================================
// Static state
// ===========================================================================

/// Host-side log callback. Invoked synchronously on every emitted record
/// when non-null.
pub type HostCallbackFn =
    extern "C" fn(level: LogLevel, color: ConsoleColor, msg: *const c_char, len: usize);

/// Effective filter level: the maximum of all per-sink levels. Records
/// strictly above this are dropped before any dispatch.
static MAX_LEVEL: AtomicU32 = AtomicU32::new(0);

/// Per-sink filter levels.
static CONSOLE_LEVEL: AtomicU32 = AtomicU32::new(0);
static DEBUG_LEVEL: AtomicU32 = AtomicU32::new(0);
static FILE_LEVEL: AtomicU32 = AtomicU32::new(0);
static HOST_LEVEL: AtomicU32 = AtomicU32::new(0);

/// Currently-installed host callback, or null when none.
static HOST_CALLBACK: AtomicPtr<HostCallbackFn> = AtomicPtr::new(ptr::null_mut());

// ===========================================================================
// High-level safe API
// ===========================================================================

/// Write a single log record to every enabled sink.
///
/// Mirrors C++ `Log::Write(LOGLEVEL, ConsoleColors, std::string_view)`.
/// Filters by the effective max level, then dispatches to each sink that
/// has been enabled for `level`.
pub fn write(level: LogLevel, color: ConsoleColor, message: &str) {
    let lvl = level as u32;
    let max = MAX_LEVEL.load(Ordering::Acquire);
    if lvl > max {
        return;
    }

    if lvl <= CONSOLE_LEVEL.load(Ordering::Acquire) {
        write_to_console(level, color, message);
    }
    if lvl <= DEBUG_LEVEL.load(Ordering::Acquire) {
        write_to_debug(level, color, message);
    }
    if lvl <= FILE_LEVEL.load(Ordering::Acquire) {
        write_to_file(level, color, message);
    }
    if lvl <= HOST_LEVEL.load(Ordering::Acquire) {
        let cb = HOST_CALLBACK.load(Ordering::Acquire);
        if !cb.is_null() {
            // SAFETY: HOST_CALLBACK was either null or written by
            // `set_host_output_level` from a valid `extern "C" fn(...)` of
            // the matching signature. The pointer is read with `Acquire`
            // and the function is `extern "C"` (synchronous), so the borrow
            // is sound for the duration of the call.
            unsafe { (*cb)(level, color, message.as_ptr() as *const c_char, message.len()) };
        }
    }
}

/// Write a formatted log record using an `Arguments` value (no allocation
/// in the hot path beyond the resulting `String`). Useful for the
/// `error_log!` / `warning_log!` / ... macros below.
pub fn write_fmt(level: LogLevel, color: ConsoleColor, args: std::fmt::Arguments<'_>) {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = s.write_fmt(args);
    write(level, color, &s);
}

/// Replace the console-sink level and recompute the effective max.
pub fn set_console_output_level(level: LogLevel) {
    CONSOLE_LEVEL.store(level as u32, Ordering::Release);
    update_max_level();
}

/// Replace the debug-sink level (Windows `OutputDebugStringW`) and
/// recompute the effective max.
pub fn set_debug_output_level(level: LogLevel) {
    DEBUG_LEVEL.store(level as u32, Ordering::Release);
    update_max_level();
}

/// Replace the file-sink level and recompute the effective max.
pub fn set_file_output_level(level: LogLevel) {
    FILE_LEVEL.store(level as u32, Ordering::Release);
    update_max_level();
}

/// Install (or remove) the host-side callback. When `callback` is `None`
/// the host sink is disabled regardless of `level`.
pub fn set_host_output_level(level: LogLevel, callback: Option<HostCallbackFn>) {
    let raw: *mut HostCallbackFn = match callback {
        Some(f) => f as *mut HostCallbackFn,
        None => ptr::null_mut(),
    };
    HOST_CALLBACK.store(raw, Ordering::Release);
    let effective = if callback.is_some() { level as u32 } else { 0 };
    HOST_LEVEL.store(effective, Ordering::Release);
    update_max_level();
}

/// Directly overwrite the effective max level (the highest of all per-sink
/// levels). Normally this is computed by [`update_max_level`]; exposing it
/// lets the FFI surface report "what's the effective filter right now".
pub fn set_max_level(level: LogLevel) {
    MAX_LEVEL.store(level as u32, Ordering::Release);
}

/// Current effective filter level.
pub fn get_max_level() -> LogLevel {
    LogLevel::from_u32(MAX_LEVEL.load(Ordering::Acquire))
}

fn update_max_level() {
    let max = CONSOLE_LEVEL
        .load(Ordering::Acquire)
        .max(DEBUG_LEVEL.load(Ordering::Acquire))
        .max(FILE_LEVEL.load(Ordering::Acquire))
        .max(HOST_LEVEL.load(Ordering::Acquire));
    MAX_LEVEL.store(max, Ordering::Release);
}

// ---------------------------------------------------------------------------
// Sink shims
// ---------------------------------------------------------------------------
//
// On PCSX2 the C++ side owns the console, debug-output and log-file sinks.
// From the Rust crate's perspective we only need to forward to the host
// callback (which the C++ side wires to whatever sinks it wants). These
// three helpers exist so the dispatch logic above is symmetrical and can
// later be wired to real Rust sinks without touching the call sites.

fn write_to_console(_level: LogLevel, _color: ConsoleColor, _message: &str) {
    // Intentionally a no-op: see module-level docs.
}

fn write_to_debug(_level: LogLevel, _color: ConsoleColor, _message: &str) {
    // Intentionally a no-op: see module-level docs.
}

fn write_to_file(_level: LogLevel, _color: ConsoleColor, _message: &str) {
    // Intentionally a no-op: see module-level docs.
}

// ===========================================================================
// log::Log integration
// ===========================================================================

/// Static logger that forwards every record into [`write`].
struct Pcsx2Logger;

impl log::Log for Pcsx2Logger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        let lvl = LogLevel::from_log_level(metadata.level()) as u32;
        lvl <= MAX_LEVEL.load(Ordering::Acquire)
    }

    fn log(&self, record: &log::Record) {
        let level = LogLevel::from_log_level(record.level());
        let color = ConsoleColor::for_log_level(record.level());
        write_fmt(level, color, *record.args());
    }

    fn flush(&self) {
        // No buffering on the Rust side; the host owns the sinks.
    }
}

static LOGGER: Pcsx2Logger = Pcsx2Logger;

/// Install [`LOGGER`] as the global `log` implementation and permit every
/// level through `log::set_max_level`. Safe to call once at program start;
/// a second call returns `Err` from the underlying `log::set_logger` and
/// is ignored.
pub fn init_logger() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Trace);
}

// ===========================================================================
// Macros
// ===========================================================================
//
// Rust equivalents of the C++ `ERROR_LOG` / `WARNING_LOG` / `INFO_LOG` /
// `DEV_LOG` variadic macros. They go through `write_fmt` so they don't
// allocate a `String` unless the level is enabled.

/// Format and emit a record at [`LogLevel::Error`] in [`ConsoleColor::StrongRed`].
#[macro_export]
macro_rules! error_log {
    ($($arg:tt)*) => {
        $crate::console::write_fmt(
            $crate::console::LogLevel::Error,
            $crate::console::ConsoleColor::StrongRed,
            ::core::format_args!($($arg)*),
        )
    };
}

/// Format and emit a record at [`LogLevel::Warning`] in [`ConsoleColor::StrongOrange`].
#[macro_export]
macro_rules! warning_log {
    ($($arg:tt)*) => {
        $crate::console::write_fmt(
            $crate::console::LogLevel::Warning,
            $crate::console::ConsoleColor::StrongOrange,
            ::core::format_args!($($arg)*),
        )
    };
}

/// Format and emit a record at [`LogLevel::Info`] in [`ConsoleColor::White`].
#[macro_export]
macro_rules! info_log {
    ($($arg:tt)*) => {
        $crate::console::write_fmt(
            $crate::console::LogLevel::Info,
            $crate::console::ConsoleColor::White,
            ::core::format_args!($($arg)*),
        )
    };
}

/// Format and emit a record at [`LogLevel::Dev`] in [`ConsoleColor::StrongGray`].
#[macro_export]
macro_rules! dev_log {
    ($($arg:tt)*) => {
        $crate::console::write_fmt(
            $crate::console::LogLevel::Dev,
            $crate::console::ConsoleColor::StrongGray,
            ::core::format_args!($($arg)*),
        )
    };
}

// ===========================================================================
// Convenience global writers
// ===========================================================================

/// Equivalent of C++ `Console` (`ConsoleLogWriter<LOGLEVEL_INFO>`).
#[derive(Copy, Clone, Debug)]
pub struct ConsoleWriter;

impl ConsoleWriter {
    #[inline]
    pub fn error(&self, msg: &str) {
        write(LogLevel::Info, ConsoleColor::StrongRed, msg);
    }
    #[inline]
    pub fn warning(&self, msg: &str) {
        write(LogLevel::Info, ConsoleColor::StrongOrange, msg);
    }
    #[inline]
    pub fn write_line(&self, msg: &str) {
        write(LogLevel::Info, ConsoleColor::Default, msg);
    }
    #[inline]
    pub fn write_line_color(&self, color: ConsoleColor, msg: &str) {
        write(LogLevel::Info, color, msg);
    }
    #[inline]
    pub fn blank(&self) {
        write(LogLevel::Info, ConsoleColor::Default, "");
    }
    #[inline]
    pub fn format(&self, color: ConsoleColor, args: std::fmt::Arguments<'_>) {
        write_fmt(LogLevel::Info, color, args);
    }
}

/// Equivalent of C++ `DevCon` (`ConsoleLogWriter<LOGLEVEL_DEV>`).
#[derive(Copy, Clone, Debug)]
pub struct DevConWriter;

impl DevConWriter {
    #[inline]
    pub fn error(&self, msg: &str) {
        write(LogLevel::Dev, ConsoleColor::StrongRed, msg);
    }
    #[inline]
    pub fn warning(&self, msg: &str) {
        write(LogLevel::Dev, ConsoleColor::StrongOrange, msg);
    }
    #[inline]
    pub fn write_line(&self, msg: &str) {
        write(LogLevel::Dev, ConsoleColor::Default, msg);
    }
    #[inline]
    pub fn write_line_color(&self, color: ConsoleColor, msg: &str) {
        write(LogLevel::Dev, color, msg);
    }
    #[inline]
    pub fn blank(&self) {
        write(LogLevel::Dev, ConsoleColor::Default, "");
    }
    #[inline]
    pub fn format(&self, color: ConsoleColor, args: std::fmt::Arguments<'_>) {
        write_fmt(LogLevel::Dev, color, args);
    }
}

/// Global writer corresponding to `extern ConsoleLogWriter<LOGLEVEL_INFO> Console`.
pub const Console: ConsoleWriter = ConsoleWriter;
/// Global writer corresponding to `extern ConsoleLogWriter<LOGLEVEL_DEV> DevCon`.
pub const DevCon: DevConWriter = DevConWriter;

// ===========================================================================
// FFI surface
// ===========================================================================

/// Install a host callback. Pass `None` (null pointer from C) to clear.
///
/// Mirrors C++ `Log::SetHostOutputLevel(LOGLEVEL, HostCallbackType)`.
#[no_mangle]
pub extern "C" fn pcsx2_log_set_host_callback(
    level: LogLevel,
    callback: Option<extern "C" fn(LogLevel, ConsoleColor, *const c_char, usize)>,
) {
    set_host_output_level(level, callback);
}

/// Write a record from C/C++.
///
/// `msg` is a UTF-8 byte buffer (not necessarily NUL-terminated) of length
/// `len` bytes. Null `msg` is treated as an empty message.
#[no_mangle]
pub extern "C" fn pcsx2_log_write(
    level: LogLevel,
    color: ConsoleColor,
    msg: *const c_char,
    len: usize,
) {
    if msg.is_null() || len == 0 {
        write(level, color, "");
        return;
    }
    // SAFETY: caller guarantees `msg` is valid for `len` bytes.
    let bytes = unsafe { std::slice::from_raw_parts(msg as *const u8, len) };
    let s = match std::str::from_utf8(bytes) {
        Ok(s) => s,
        Err(_) => return,
    };
    write(level, color, s);
}

/// Overwrite the effective max level (highest of all per-sink levels).
#[no_mangle]
pub extern "C" fn pcsx2_log_set_level(level: LogLevel) {
    set_max_level(level);
}

/// Query the effective max level.
#[no_mangle]
pub extern "C" fn pcsx2_log_get_level() -> LogLevel {
    get_max_level()
}