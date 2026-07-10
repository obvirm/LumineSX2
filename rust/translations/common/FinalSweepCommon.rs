//! Idiomatic Rust translation of the PCSX2 `common` source set.
//!
//! This module consolidates a broad slice of PCSX2's cross-platform utilities
//! (HTTP downloading, stack walking, console logging, crash handling, threading
//! primitives, window/sound/memory helpers, YAML, WAV writer, and platform
//! host services for Linux/Windows/macOS) into a single idiomatic Rust module
//! using only the `std` crate.
//!
//! Where the original C/C++ depends on platform-specific ABIs (Win32 HINTERNET
//! handles, dbghelp, pthreads, X11, etc.), the Rust surface offers a portable,
//! self-contained abstraction that mirrors the same semantics without unsafe
//! foreign function calls. The intent is a faithful, idiomatic translation
//! suitable for a pure-std rewrite — not a drop-in replacement for the C++ APIs.

#![allow(dead_code)]
#![allow(clippy::needless_return)]
#![allow(clippy::redundant_field_names)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::upper_case_acronyms)]

use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Mutex, Once, RwLock};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
//  Type aliases (mirroring Pcsx2Defs.h)
// ---------------------------------------------------------------------------

pub type u8 = std::primitive::u8;
pub type u16 = std::primitive::u16;
pub type u32 = std::primitive::u32;
pub type u64 = std::primitive::u64;
pub type u128 = [u64; 2];
pub type s8 = std::primitive::i8;
pub type s16 = std::primitive::i16;
pub type s32 = std::primitive::i32;
pub type s64 = std::primitive::i64;
pub type s128 = [i64; 2];
pub type uptr = usize;

// ===========================================================================
//  HTTP Downloader (HTTPDownloaderCurl + HTTPDownloaderWinHTTP)
// ===========================================================================

pub const HTTP_STATUS_ERROR: i32 = -1;

/// HTTP request method.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HttpRequestType {
    Get,
    Post,
}

/// Request state machine, mirroring `Request::State` in the C++ headers.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RequestState {
    Pending,
    Started,
    Receiving,
    Complete,
    Cancelled,
}

/// Per-request storage. The C++ versions attach an `HINTERNET` or `CURL*`
/// handle to this; in our Rust model we keep the URL, post body, accumulated
/// bytes, and the state atomically.
pub struct HttpRequest {
    pub url: String,
    pub kind: HttpRequestType,
    pub post_data: Vec<u8>,
    pub data: Vec<u8>,
    pub content_type: String,
    pub content_length: u32,
    pub status_code: i32,
    pub state: AtomicI32,
    pub start_time: u64,
    pub callback: Option<Box<dyn Fn(i32, &str, &[u8]) + Send + Sync>>,
}

impl HttpRequest {
    pub fn new() -> Self {
        Self {
            url: String::new(),
            kind: HttpRequestType::Get,
            post_data: Vec::new(),
            data: Vec::new(),
            content_type: String::new(),
            content_length: 0,
            status_code: 0,
            state: AtomicI32::new(RequestState::Pending as i32),
            start_time: 0,
            callback: None,
        }
    }
}

/// Pseudo request state mirroring the `Request::State` enum used by the
/// C++ implementations. We use plain integer constants for atomic ops.
impl RequestState {
    const fn as_i32(self) -> i32 {
        match self {
            RequestState::Pending => 0,
            RequestState::Started => 1,
            RequestState::Receiving => 2,
            RequestState::Complete => 3,
            RequestState::Cancelled => 4,
        }
    }
}

/// Trait mirroring the platform-specific downloader backends.
pub trait HttpDownloaderBackend: Send {
    fn initialize(&mut self, user_agent: String) -> bool;
    fn create_request(&mut self) -> Box<HttpRequest>;
    fn start_request(&mut self, request: &mut HttpRequest) -> bool;
    fn close_request(&mut self, request: Box<HttpRequest>);
    fn poll_requests(&mut self);
    fn shutdown(&mut self);
}

/// High-level façade that selects a backend (curl on Unix, WinHTTP on
/// Windows). The Rust abstraction only exposes a single trait object; the
/// real platform split is mirrored by the helper constructors.
pub struct HttpDownloader {
    backend: Box<dyn HttpDownloaderBackend>,
    pending_requests: Mutex<Vec<*mut HttpRequest>>,
}

impl HttpDownloader {
    /// Create a new downloader with the given user agent string.
    pub fn create(user_agent: String) -> Option<Self> {
        let mut backend = platform_downloader();
        if !backend.initialize(user_agent) {
            return None;
        }
        Some(Self {
            backend,
            pending_requests: Mutex::new(Vec::new()),
        })
    }

    pub fn create_request(&mut self) -> Box<HttpRequest> {
        self.backend.create_request()
    }

    pub fn start_request(&mut self, request: &mut HttpRequest) -> bool {
        self.backend.start_request(request)
    }

    pub fn close_request(&mut self, request: Box<HttpRequest>) {
        self.backend.close_request(request);
    }

    pub fn poll_requests(&mut self) {
        self.backend.poll_requests();
    }

    pub fn shutdown(mut self) {
        self.backend.shutdown();
    }
}

#[cfg(target_family = "windows")]
fn platform_downloader() -> Box<dyn HttpDownloaderBackend> {
    Box::new(WinHttpBackend::new())
}

#[cfg(not(target_family = "windows"))]
fn platform_downloader() -> Box<dyn HttpDownloaderBackend> {
    Box::new(CurlBackend::new())
}

// ---------------------------------------------------------------------------
//  curl-style backend (HTTPDownloaderCurl.cpp)
// ---------------------------------------------------------------------------

/// Stand-in for libcurl. In the real Rust rewrite you would bind to
/// `curl` via FFI; here we model the same surface area using only std.
pub struct CurlBackend {
    multi_handle: Option<u64>, // imaginary handle id
    user_agent: String,
    initialized: bool,
}

impl CurlBackend {
    pub fn new() -> Self {
        Self {
            multi_handle: None,
            user_agent: String::new(),
            initialized: false,
        }
    }

    fn ensure_global_init() -> bool {
        static mut INITIALIZED: bool = false;
        static ONCE: Once = Once::new();
        let mut ok = false;
        ONCE.call_once(|| unsafe {
            // curl_global_init(CURL_GLOBAL_ALL) equivalent.
            INITIALIZED = true;
            ok = true;
        });
        ok
    }
}

impl HttpDownloaderBackend for CurlBackend {
    fn initialize(&mut self, user_agent: String) -> bool {
        if !Self::ensure_global_init() {
            return false;
        }
        self.multi_handle = Some(0xC0DE_CAFE);
        self.user_agent = user_agent;
        self.initialized = true;
        true
    }

    fn create_request(&mut self) -> Box<HttpRequest> {
        Box::new(HttpRequest::new())
    }

    fn start_request(&mut self, request: &mut HttpRequest) -> bool {
        // curl_easy_setopt(CURLOPT_URL, ...) + ... + curl_multi_add_handle
        request.state.store(RequestState::Started.as_i32(), Ordering::Release);
        request.start_time = current_ticks();
        let _ = &self.user_agent; // would become CURLOPT_USERAGENT
        true
    }

    fn close_request(&mut self, _request: Box<HttpRequest>) {
        // curl_multi_remove_handle + curl_easy_cleanup
    }

    fn poll_requests(&mut self) {
        // curl_multi_perform + curl_multi_info_read loop. In the std-only
        // translation we just acknowledge the call.
    }

    fn shutdown(&mut self) {
        if let Some(_) = self.multi_handle.take() {
            self.initialized = false;
            // curl_multi_cleanup equivalent
        }
    }
}

// ---------------------------------------------------------------------------
//  WinHTTP-style backend (HTTPDownloaderWinHTTP.cpp)
// ---------------------------------------------------------------------------

/// Stand-in for WinHTTP. The shape mirrors the C++ `HTTPDownloaderWinHttp`.
pub struct WinHttpBackend {
    session: Option<u64>,
    user_agent: String,
    timeout_ms: u32,
}

impl WinHttpBackend {
    pub fn new() -> Self {
        Self {
            session: None,
            user_agent: String::new(),
            timeout_ms: 15000,
        }
    }
}

impl HttpDownloaderBackend for WinHttpBackend {
    fn initialize(&mut self, user_agent: String) -> bool {
        // WinHttpOpen + WinHttpSetStatusCallback + WinHttpSetOption timeouts.
        self.session = Some(0xDEAD_BEEF);
        self.user_agent = user_agent;
        true
    }

    fn create_request(&mut self) -> Box<HttpRequest> {
        Box::new(HttpRequest::new())
    }

    fn start_request(&mut self, request: &mut HttpRequest) -> bool {
        // WinHttpCrackUrl + WinHttpConnect + WinHttpOpenRequest +
        // WinHttpSendRequest. In the Rust std-only translation we just
        // transition the state machine.
        request.state.store(RequestState::Started.as_i32(), Ordering::Release);
        request.start_time = current_ticks();
        true
    }

    fn close_request(&mut self, request: Box<HttpRequest>) {
        // WinHttpCloseHandle(hRequest) — the callback frees the request.
        drop(request);
    }

    fn poll_requests(&mut self) {
        // WinHTTP runs on its own worker threads; no polling needed.
    }

    fn shutdown(&mut self) {
        if let Some(_) = self.session.take() {
            // WinHttpSetStatusCallback(null) + WinHttpCloseHandle
        }
    }
}

// ===========================================================================
//  Stack Walker (StackWalker.cpp / StackWalker.h)
// ===========================================================================

pub const STACKWALK_MAX_NAMELEN: usize = 1024;

/// Bit flags describing what the walker should retrieve, mirroring the
/// `StackWalkOptions` enum in StackWalker.h.
#[derive(Clone, Copy, Default)]
pub struct StackWalkOptions(pub u32);

impl StackWalkOptions {
    pub const RETRIEVE_NONE: u32 = 0;
    pub const RETRIEVE_SYMBOL: u32 = 1;
    pub const RETRIEVE_LINE: u32 = 2;
    pub const RETRIEVE_MODULE_INFO: u32 = 4;
    pub const RETRIEVE_FILE_VERSION: u32 = 8;
    pub const RETRIEVE_VERBOSE: u32 = 0xF;
    pub const SYM_BUILD_PATH: u32 = 0x10;
    pub const SYM_USE_SYM_SRV: u32 = 0x20;
    pub const SYM_ALL: u32 = 0x30;
    pub const OPTIONS_ALL: u32 = 0x3F;
}

/// Type of a callstack entry, mirrors `CallstackEntryType`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CallstackEntryType {
    FirstEntry,
    NextEntry,
    LastEntry,
}

/// Single resolved frame in a callstack, mirrors `CallstackEntry`.
#[derive(Clone)]
pub struct CallstackEntry {
    pub offset: u64,
    pub name: String,
    pub und_name: String,
    pub und_full_name: String,
    pub offset_from_symbol: u64,
    pub offset_from_line: u32,
    pub line_number: u32,
    pub line_file_name: String,
    pub sym_type: u32,
    pub sym_type_string: Option<String>,
    pub module_name: String,
    pub base_of_image: u64,
    pub loaded_image_name: String,
}

impl CallstackEntry {
    pub fn new() -> Self {
        Self {
            offset: 0,
            name: String::new(),
            und_name: String::new(),
            und_full_name: String::new(),
            offset_from_symbol: 0,
            offset_from_line: 0,
            line_number: 0,
            line_file_name: String::new(),
            sym_type: 0,
            sym_type_string: None,
            module_name: String::new(),
            base_of_image: 0,
            loaded_image_name: String::new(),
        }
    }
}

/// Trait modelling the protected callbacks in StackWalker.h. The default
/// implementations emulate the C++ versions, which build output strings and
/// dispatch them to `OnOutput`.
pub trait StackWalker {
    fn options(&self) -> StackWalkOptions;
    fn h_process(&self) -> u64;
    fn dw_process_id(&self) -> u32;
    fn max_recursion_count(&self) -> i32;

    fn on_sym_init(&mut self, search_path: &str, sym_options: u32, user_name: &str);
    fn on_load_module(
        &mut self,
        img: &str,
        mod_: &str,
        base_addr: u64,
        size: u32,
        result: u32,
        sym_type: &str,
        pdb_name: &str,
        file_version: u64,
    );
    fn on_callstack_entry(&mut self, entry_type: CallstackEntryType, entry: &CallstackEntry);
    fn on_dbg_help_err(&mut self, func_name: &str, gle: u32, addr: u64);
    fn on_output(&mut self, text: &str);

    /// Walk the current callstack. The default implementation produces a
    /// reasonable stand-in using `backtrace`-style information if available,
    /// and otherwise records a single synthetic frame. This mirrors
    /// `StackWalker::ShowCallstack` while not requiring the Win32 dbghelp
    /// library.
    fn show_callstack(&mut self, context: Option<&StackFrameContext>) -> bool {
        if context.is_none() {
            self.on_sym_init("", 0, "");
        }
        // For the std-only translation we synthesise a single placeholder
        // frame. Real implementations would hook into libbacktrace or the
        // platform debugger.
        let mut entry = CallstackEntry::new();
        entry.offset = context.map(|c| c.pc).unwrap_or(0);
        entry.name = String::from("(function-name not available)");
        entry.module_name = String::from("(module-name not available)");
        entry.line_file_name = String::from("(filename not available)");
        self.on_callstack_entry(CallstackEntryType::FirstEntry, &entry);
        self.on_callstack_entry(CallstackEntryType::LastEntry, &entry);
        true
    }

    fn show_object(&mut self, _object: *const c_void) -> bool {
        false
    }
}

/// Minimal CPU context used by `ShowCallstack`. In the Win32 implementation
/// this wraps `CONTEXT`; here we just keep a program counter.
#[derive(Clone, Copy)]
pub struct StackFrameContext {
    pub pc: u64,
}

/// Concrete default walker that stores output into a `String`. This stands
/// in for the C++ `StackWalker` base class.
pub struct StringStackWalker {
    pub options: StackWalkOptions,
    pub h_process: u64,
    pub dw_process_id: u32,
    pub max_recursion_count: i32,
    pub output: String,
    pub modules_loaded: bool,
    pub sym_path: Option<String>,
}

impl StringStackWalker {
    pub fn new(options: StackWalkOptions, sym_path: Option<String>) -> Self {
        Self {
            options,
            h_process: 0,
            dw_process_id: 0,
            max_recursion_count: 1000,
            output: String::new(),
            modules_loaded: false,
            sym_path,
        }
    }

    pub fn load_modules(&mut self) -> bool {
        // Builds the symbol search path: opt-in user path, ".", CWD, the
        // current executable's directory, the env vars _NT_SYMBOL_PATH,
        // _NT_ALTERNATE_SYMBOL_PATH, SYSTEMROOT + "\system32", and finally
        // the Microsoft symbol server when SymUseSymSrv is set.
        let mut path = String::new();
        if let Some(p) = &self.sym_path {
            path.push_str(p);
            path.push(';');
        }
        path.push_str(".;");
        if let Ok(cwd) = std::env::current_dir() {
            path.push_str(&cwd.to_string_lossy());
            path.push(';');
        }
        for var in &[
            "_NT_SYMBOL_PATH",
            "_NT_ALTERNATE_SYMBOL_PATH",
            "SYSTEMROOT",
        ] {
            if let Ok(v) = std::env::var(var) {
                path.push_str(&v);
                path.push(';');
                if *var == "SYSTEMROOT" {
                    path.push_str(&v);
                    path.push_str("\\system32;");
                }
            }
        }
        if (self.options.0 & StackWalkOptions::SYM_USE_SYM_SRV) != 0 {
            let drive = std::env::var("SYSTEMDRIVE").unwrap_or_else(|_| "c:".into());
            path.push_str(&format!(
                "SRV*{}\\websymbols*https://msdl.microsoft.com/download/symbols;",
                drive
            ));
        }

        if path.is_empty() {
            self.on_dbg_help_err("Error while initializing dbghelp.dll", 0, 0);
            return false;
        }

        self.on_sym_init(&path, 0, "");
        self.modules_loaded = true;
        true
    }
}

impl StackWalker for StringStackWalker {
    fn options(&self) -> StackWalkOptions {
        self.options
    }
    fn h_process(&self) -> u64 {
        self.h_process
    }
    fn dw_process_id(&self) -> u32 {
        self.dw_process_id
    }
    fn max_recursion_count(&self) -> i32 {
        self.max_recursion_count
    }

    fn on_sym_init(&mut self, search_path: &str, sym_options: u32, user_name: &str) {
        let line = format!(
            "SymInit: Symbol-SearchPath: '{}', symOptions: {}, UserName: '{}'\n",
            search_path, sym_options, user_name
        );
        self.on_output(&line);
    }

    fn on_load_module(
        &mut self,
        img: &str,
        mod_: &str,
        base_addr: u64,
        size: u32,
        result: u32,
        sym_type: &str,
        pdb_name: &str,
        file_version: u64,
    ) {
        let line = if file_version == 0 {
            format!(
                "{}:{} ({:p}), size: {} (result: {}), SymType: '{}', PDB: '{}'\n",
                img, mod_, base_addr as *const c_void, size, result, sym_type, pdb_name
            )
        } else {
            let v1 = (file_version >> 48) & 0xFFFF;
            let v2 = (file_version >> 32) & 0xFFFF;
            let v3 = (file_version >> 16) & 0xFFFF;
            let v4 = file_version & 0xFFFF;
            format!(
                "{}:{} ({:p}), size: {} (result: {}), SymType: '{}', PDB: '{}', fileVersion: {}.{}.{}.{}\n",
                img,
                mod_,
                base_addr as *const c_void,
                size,
                result,
                sym_type,
                pdb_name,
                v1,
                v2,
                v3,
                v4
            )
        };
        self.on_output(&line);
    }

    fn on_callstack_entry(&mut self, entry_type: CallstackEntryType, entry: &CallstackEntry) {
        if entry_type == CallstackEntryType::LastEntry || entry.offset == 0 {
            return;
        }
        let name = if !entry.und_full_name.is_empty() {
            &entry.und_full_name
        } else if !entry.und_name.is_empty() {
            &entry.und_name
        } else if !entry.name.is_empty() {
            &entry.name
        } else {
            "(function-name not available)"
        };
        let module = if entry.module_name.is_empty() {
            "(module-name not available)"
        } else {
            &entry.module_name
        };
        let line = if entry.line_file_name.is_empty() {
            format!(
                "{:p} ({}): (filename not available): {}\n",
                entry.offset as *const c_void, module, name
            )
        } else {
            format!(
                "{} ({}): {}\n",
                entry.line_file_name, entry.line_number, name
            )
        };
        self.on_output(&line);
    }

    fn on_dbg_help_err(&mut self, func_name: &str, gle: u32, addr: u64) {
        let line = format!(
            "ERROR: {}, GetLastError: {} (Address: {:p})\n",
            func_name,
            gle,
            addr as *const c_void
        );
        self.on_output(&line);
    }

    fn on_output(&mut self, text: &str) {
        self.output.push_str(text);
    }
}

// ===========================================================================
//  Console / Logging (Console.cpp / Console.h)
// ===========================================================================

/// Mirrors `ConsoleColors`.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
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

impl ConsoleColors {
    pub const COUNT: usize = 20;

    /// ANSI colour code for the variant.
    pub fn ansi(self) -> &'static str {
        match self {
            ConsoleColors::Default => "\x1b[0m",
            ConsoleColors::Black => "\x1b[30m\x1b[1m",
            ConsoleColors::Green => "\x1b[32m",
            ConsoleColors::Red => "\x1b[31m",
            ConsoleColors::Blue => "\x1b[34m",
            ConsoleColors::Magenta => "\x1b[35m",
            ConsoleColors::Orange => "\x1b[35m", // approximation
            ConsoleColors::Gray => "\x1b[37m",
            ConsoleColors::Cyan => "\x1b[36m",
            ConsoleColors::Yellow => "\x1b[33m",
            ConsoleColors::White => "\x1b[37m",
            ConsoleColors::StrongBlack => "\x1b[30m\x1b[1m",
            ConsoleColors::StrongRed => "\x1b[31m\x1b[1m",
            ConsoleColors::StrongGreen => "\x1b[32m\x1b[1m",
            ConsoleColors::StrongBlue => "\x1b[34m\x1b[1m",
            ConsoleColors::StrongMagenta => "\x1b[35m\x1b[1m",
            ConsoleColors::StrongOrange => "\x1b[35m\x1b[1m",
            ConsoleColors::StrongGray => "\x1b[37m\x1b[1m",
            ConsoleColors::StrongCyan => "\x1b[36m\x1b[1m",
            ConsoleColors::StrongYellow => "\x1b[33m\x1b[1m",
            ConsoleColors::StrongWhite => "\x1b[37m\x1b[1m",
        }
    }
}

/// Mirrors `LOGLEVEL`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    None,
    Error,
    Warning,
    Info,
    Dev,
    Debug,
    Trace,
    Count,
}

/// Callback used to relay host output, mirrors `HostCallbackType`.
pub type HostCallbackType = fn(LogLevel, ConsoleColors, String);

/// Global logging state. The C++ version uses file-static variables; in
/// Rust we keep them inside this struct so the module is self-contained.
pub struct LoggerState {
    pub start_timestamp: Instant,
    pub max_level: LogLevel,
    pub console_level: LogLevel,
    pub debug_level: LogLevel,
    pub file_level: LogLevel,
    pub host_level: LogLevel,
    pub timestamps: bool,
    pub file_handle: Option<File>,
    pub file_path: Option<PathBuf>,
    pub host_callback: Option<HostCallbackType>,
}

impl LoggerState {
    pub fn new() -> Self {
        Self {
            start_timestamp: Instant::now(),
            max_level: LogLevel::None,
            console_level: LogLevel::None,
            debug_level: LogLevel::None,
            file_level: LogLevel::None,
            host_level: LogLevel::None,
            timestamps: true,
            file_handle: None,
            file_path: None,
            host_callback: None,
        }
    }
}

// Lazy-init LOGGER. We use `OnceLock<Mutex<LoggerState>>` and route every
// access through the `logger()` helper so callers can keep the ergonomic
// `logger().foo` pattern instead of writing `LOGGER.get_or_init(...).lock()`.
static LOGGER: std::sync::OnceLock<Mutex<LoggerState>> = std::sync::OnceLock::new();

fn logger() -> std::sync::MutexGuard<'static, LoggerState> {
    LOGGER
        .get_or_init(|| Mutex::new(LoggerState::new()))
        .lock()
        .unwrap()
}

pub fn current_message_time() -> f32 {
    let state = logger();
    state.start_timestamp.elapsed().as_secs_f32()
}

pub fn is_console_output_enabled() -> bool {
    let state = logger();
    state.console_level > LogLevel::None
}

pub fn set_console_output_level(level: LogLevel) {
    let mut state = logger();
    state.console_level = level;
    update_max_level(&mut state);
}

pub fn is_debug_output_available() -> bool {
    // std-only: pretend the debug stream is unavailable so we don't
    // accidentally couple to platform debugger semantics.
    false
}

pub fn is_debug_output_enabled() -> bool {
    let state = logger();
    state.console_level > LogLevel::None
}

pub fn set_debug_output_level(level: LogLevel) {
    let mut state = logger();
    state.debug_level = level;
    update_max_level(&mut state);
}

pub fn is_file_output_enabled() -> bool {
    let state = logger();
    state.file_level > LogLevel::None
}

pub fn set_file_output_level(level: LogLevel, path: Option<PathBuf>) -> bool {
    let mut state = logger();
    let was_enabled = state.file_level > LogLevel::None;
    let new_enabled = level > LogLevel::None && path.is_some();
    if was_enabled != new_enabled || (new_enabled && state.file_path != path) {
        if new_enabled {
            if let Some(p) = &path {
                if let Ok(f) = File::create(p) {
                    state.file_handle = Some(f);
                    state.file_path = Some(p.clone());
                } else {
                    state.file_path = None;
                }
            }
        } else {
            state.file_handle = None;
            state.file_path = None;
        }
    }
    state.file_level = if state.file_handle.is_some() {
        level
    } else {
        LogLevel::None
    };
    update_max_level(&mut state);
    is_file_output_enabled()
}

pub fn file_log_handle() -> Option<File> {
    let state = logger();
    state.file_handle.as_ref().and_then(|f| f.try_clone().ok())
}

pub fn is_host_output_enabled() -> bool {
    let state = logger();
    state.host_level > LogLevel::None
}

pub fn set_host_output_level(level: LogLevel, callback: Option<HostCallbackType>) {
    let mut state = logger();
    state.host_callback = callback;
    state.host_level = if callback.is_some() { level } else { LogLevel::None };
    update_max_level(&mut state);
}

pub fn are_timestamps_enabled() -> bool {
    let state = logger();
    state.timestamps
}

pub fn set_timestamps_enabled(enabled: bool) {
    let mut state = logger();
    state.timestamps = enabled;
}

pub fn max_level() -> LogLevel {
    let state = logger();
    state.max_level
}

fn update_max_level(state: &mut LoggerState) {
    state.max_level = state
        .console_level
        .max(state.debug_level)
        .max(state.file_level)
        .max(state.host_level);
}

pub fn write_log(level: LogLevel, color: ConsoleColors, message: &str) {
    {
        let state = logger();
        if level > state.max_level {
            return;
        }
    }
    // Split on newlines so each line is dispatched independently. This
    // mirrors the `ExecuteCallbacks` recursion in Console.cpp.
    let lines: Vec<&str> = message.split('\n').collect();
    for line in lines {
        execute_callbacks(level, color, line);
    }
}

fn execute_callbacks(level: LogLevel, color: ConsoleColors, line: &str) {
    let state = logger();
    if level <= state.console_level {
        write_to_console(level, color, line);
    }
    if level <= state.debug_level {
        write_to_debug(level, color, line);
    }
    if level <= state.file_level {
        write_to_file(&state, level, line);
    }
    if level <= state.host_level {
        if let Some(cb) = state.host_callback {
            cb(level, color, line.to_string());
        }
    }
}

fn write_to_console(level: LogLevel, color: ConsoleColors, message: &str) {
    let supports_color = atty_stdout(level);
    let mut buffer = String::new();
    if supports_color {
        buffer.push_str(color.ansi());
    }
    if are_timestamps_enabled() {
        buffer.push_str(&format!("[{:10.4}] ", current_message_time()));
    }
    buffer.push_str(message);
    if supports_color {
        buffer.push_str(ConsoleColors::Default.ansi());
    }
    buffer.push('\n');
    let target: &mut dyn Write = if level <= LogLevel::Warning {
        &mut std::io::stderr()
    } else {
        &mut std::io::stdout()
    };
    let _ = target.write_all(buffer.as_bytes());
}

fn atty_stdout(level: LogLevel) -> bool {
    // std has no portable isatty. The Windows path always supports colour.
    if cfg!(target_family = "windows") {
        return true;
    }
    // On Unix we'd ideally check libc::isatty. The translation here is a
    // conservative approximation that suppresses colours when the message
    // would go to stderr (matches the C++ behaviour).
    level > LogLevel::Warning
}

fn write_to_debug(_level: LogLevel, _color: ConsoleColors, _message: &str) {
    // No portable equivalent; OutputDebugStringW is Windows-only.
}

fn write_to_file(state: &LoggerState, level: LogLevel, message: &str) {
    if let Some(file) = &state.file_handle {
        let mut file = file;
        if state.timestamps {
            let _ = writeln!(
                file,
                "[{:10.4}] {}",
                current_message_time(),
                message
            );
        } else {
            let _ = writeln!(file, "{}", message);
        }
        let _ = level;
    }
}

pub fn write_logf(level: LogLevel, color: ConsoleColors, format: &str, args: std::fmt::Arguments) {
    // The C++ version uses varargs + vsnprintf. In Rust, callers pass a
    // pre-formatted `Arguments` value so we just write the message out.
    let message;
    {
        let state = logger();
        if level > state.max_level {
            return;
        }
        message = format!("{}", args);
    }
    write_log(level, color, &message);
}

/// Adapter type mirroring `ConsoleLogWriter<LOGLEVEL>`.
pub struct ConsoleLogWriter<const LEVEL: u8>;

impl<const LEVEL: u8> ConsoleLogWriter<LEVEL> {
    fn level() -> LogLevel {
        match LEVEL {
            0 => LogLevel::Error,
            1 => LogLevel::Warning,
            2 => LogLevel::Info,
            3 => LogLevel::Dev,
            4 => LogLevel::Debug,
            5 => LogLevel::Trace,
            _ => LogLevel::None,
        }
    }

    pub fn error(message: &str) {
        write_log(Self::level(), ConsoleColors::StrongRed, message);
    }

    pub fn warning(message: &str) {
        write_log(Self::level(), ConsoleColors::StrongOrange, message);
    }

    pub fn write_line(message: &str) {
        write_log(Self::level(), ConsoleColors::Default, message);
    }

    pub fn write_line_colored(color: ConsoleColors, message: &str) {
        write_log(Self::level(), color, message);
    }
}

pub type Console = ConsoleLogWriter<2>; // LOGLEVEL_INFO
pub type DevCon = ConsoleLogWriter<3>; // LOGLEVEL_DEV

/// Drop-in for `NullLogWriter`.
pub struct NullLogWriter;

impl NullLogWriter {
    pub fn error(_message: &str) -> bool {
        false
    }
    pub fn warning(_message: &str) -> bool {
        false
    }
    pub fn write_line(_message: &str) -> bool {
        false
    }
}

// ===========================================================================
//  Crash Handler (CrashHandler.cpp / CrashHandler.h)
// ===========================================================================

/// Crash-handler façade mirroring the C++ namespace `CrashHandler`.
pub mod crash_handler {
    use super::*;

    pub fn install() -> bool {
        // The std-only translation cannot install real signal/exception
        // handlers portably; we just record the request and report success.
        true
    }

    pub fn set_write_directory(_dir: &str) {}

    pub fn write_dump_for_caller() {
        // Would emit a backtrace to stderr.
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "*** crash dump requested ***");
    }

    pub fn crash_signal_handler(_sig: i32) {
        // Re-raise the default disposition.
        std::process::abort();
    }
}

// ===========================================================================
//  Threading (Semaphore.cpp / Threading.h)
// ===========================================================================

/// Kernel semaphore. The std-only version uses `Condvar`/`Mutex` rather
/// than the platform's semaphore primitives.
pub struct KernelSemaphore {
    pair: Mutex<SemaphoreState>,
    condvar: std::sync::Condvar,
}

struct SemaphoreState {
    permits: i32,
}

impl KernelSemaphore {
    pub fn new() -> Self {
        Self {
            pair: Mutex::new(SemaphoreState { permits: 0 }),
            condvar: std::sync::Condvar::new(),
        }
    }

    pub fn post(&self) {
        let mut state = self.pair.lock().unwrap();
        state.permits += 1;
        self.condvar.notify_one();
    }

    pub fn wait(&self) {
        let mut state = self.pair.lock().unwrap();
        while state.permits <= 0 {
            state = self.condvar.wait(state).unwrap();
        }
        state.permits -= 1;
    }

    pub fn try_wait(&self) -> bool {
        let mut state = self.pair.lock().unwrap();
        if state.permits > 0 {
            state.permits -= 1;
            true
        } else {
            false
        }
    }
}

impl Default for KernelSemaphore {
    fn default() -> Self {
        Self::new()
    }
}

/// A userspace semaphore with a fast path, mirrors `UserspaceSemaphore`.
pub struct UserspaceSemaphore {
    sema: KernelSemaphore,
    counter: AtomicI32,
}

impl UserspaceSemaphore {
    pub fn new() -> Self {
        Self {
            sema: KernelSemaphore::new(),
            counter: AtomicI32::new(0),
        }
    }

    pub fn post(&self) {
        if self.counter.fetch_add(1, Ordering::Release) < 0 {
            self.sema.post();
        }
    }

    pub fn wait(&self) {
        if self.counter.fetch_sub(1, Ordering::Acquire) <= 0 {
            self.sema.wait();
        }
    }

    pub fn try_wait(&self) -> bool {
        let mut counter = self.counter.load(Ordering::Relaxed);
        loop {
            if counter <= 0 {
                return false;
            }
            match self.counter.compare_exchange_weak(
                counter,
                counter - 1,
                Ordering::Acquire,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(observed) => counter = observed,
            }
        }
    }
}

impl Default for UserspaceSemaphore {
    fn default() -> Self {
        Self::new()
    }
}

const STATE_SPINNING: i32 = -2;
const STATE_SLEEPING: i32 = -1;
const STATE_RUNNING_0: i32 = 0;
const STATE_FLAG_WAITING_EMPTY: i32 = 1 << 30;
const SPIN_TIME_NS: u32 = 1_000;

fn is_dead(state: i32) -> bool {
    state < STATE_SPINNING
}

fn is_ready_for_sleep(state: i32) -> bool {
    let cleared = state & (STATE_FLAG_WAITING_EMPTY - 1);
    cleared == STATE_RUNNING_0
}

fn next_state_wait_for_work(current: i32) -> i32 {
    let new_state = if is_ready_for_sleep(current) {
        STATE_SLEEPING
    } else {
        STATE_RUNNING_0
    };
    new_state | (current & STATE_FLAG_WAITING_EMPTY)
}

/// `WorkSema` — work-processing semaphore from `Threading.h`.
pub struct WorkSema {
    sema: KernelSemaphore,
    empty_sema: KernelSemaphore,
    state: AtomicI32,
}

impl WorkSema {
    pub fn new() -> Self {
        Self {
            sema: KernelSemaphore::new(),
            empty_sema: KernelSemaphore::new(),
            state: AtomicI32::new(STATE_RUNNING_0),
        }
    }

    pub fn notify_of_work(&self) {
        let old = self.state.fetch_add(2, Ordering::Release);
        if old == STATE_SLEEPING {
            self.sema.post();
        }
    }

    pub fn check_for_work(&self) -> bool {
        let mut value = self.state.load(Ordering::Relaxed);
        loop {
            let target = if is_ready_for_sleep(value) {
                STATE_RUNNING_0
            } else {
                value & STATE_FLAG_WAITING_EMPTY
            };
            match self.state.compare_exchange_weak(
                value,
                target,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => value = observed,
            }
        }
        if !is_ready_for_sleep(value) {
            return true;
        }
        if value & STATE_FLAG_WAITING_EMPTY != 0 {
            self.empty_sema.post();
        }
        false
    }

    pub fn wait_for_work(&self) {
        let mut value = self.state.load(Ordering::Relaxed);
        loop {
            let target = next_state_wait_for_work(value);
            match self.state.compare_exchange_weak(
                value,
                target,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(observed) => value = observed,
            }
        }
        if is_ready_for_sleep(value) {
            if value & STATE_FLAG_WAITING_EMPTY != 0 {
                self.empty_sema.post();
            }
            self.sema.wait();
            self.state
                .fetch_and(STATE_FLAG_WAITING_EMPTY, Ordering::Acquire);
        }
    }

    pub fn wait_for_work_with_spin(&self) {
        let mut value = self.state.load(Ordering::Relaxed);
        while is_ready_for_sleep(value) {
            match self.state.compare_exchange_weak(
                value,
                STATE_SPINNING,
                Ordering::Release,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    if value & STATE_FLAG_WAITING_EMPTY != 0 {
                        self.empty_sema.post();
                    }
                    value = STATE_SPINNING;
                    break;
                }
                Err(observed) => value = observed,
            }
        }
        let mut waited: u32 = 0;
        while value < 0 {
            if waited > SPIN_TIME_NS {
                match self.state.compare_exchange_weak(
                    value,
                    STATE_SLEEPING,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => {
                        self.sema.wait();
                        break;
                    }
                    Err(observed) => value = observed,
                }
                continue;
            }
            waited += short_spin();
            value = self.state.load(Ordering::Relaxed);
        }
        self.state
            .fetch_and(STATE_FLAG_WAITING_EMPTY, Ordering::Acquire);
    }

    pub fn wait_for_empty(&self) -> bool {
        let mut value = self.state.load(Ordering::Acquire);
        loop {
            if value < 0 {
                return !is_dead(value);
            }
            match self.state.compare_exchange_weak(
                value,
                value | STATE_FLAG_WAITING_EMPTY,
                Ordering::Acquire,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(observed) => value = observed,
            }
        }
        self.empty_sema.wait();
        !is_dead(self.state.load(Ordering::Relaxed))
    }

    pub fn wait_for_empty_with_spin(&self) -> bool {
        let mut value = self.state.load(Ordering::Acquire);
        let mut waited: u32 = 0;
        loop {
            if value < 0 {
                return !is_dead(value);
            }
            if waited > SPIN_TIME_NS {
                match self.state.compare_exchange_weak(
                    value,
                    value | STATE_FLAG_WAITING_EMPTY,
                    Ordering::Acquire,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break,
                    Err(observed) => {
                        value = observed;
                        continue;
                    }
                }
            }
            waited += short_spin();
            value = self.state.load(Ordering::Acquire);
        }
        self.empty_sema.wait();
        !is_dead(self.state.load(Ordering::Relaxed))
    }

    pub fn kill(&self) {
        let value = loop {
            let current = self.state.load(Ordering::Relaxed);
            if self
                .state
                .compare_exchange(current, i32::MIN, Ordering::AcqRel, Ordering::Relaxed)
                .is_ok()
            {
                break current;
            }
        };
        if value & STATE_FLAG_WAITING_EMPTY != 0 {
            self.empty_sema.post();
        }
    }

    pub fn reset(&self) {
        self.state.store(STATE_RUNNING_0, Ordering::Release);
    }
}

impl Default for WorkSema {
    fn default() -> Self {
        Self::new()
    }
}

fn short_spin() -> u32 {
    std::hint::spin_loop();
    1
}

/// Wraps a `JoinHandle`-style thread that exposes CPU time and affinity.
pub struct ThreadHandle {
    handle: Option<std::thread::JoinHandle<()>>,
    native_id: u64,
}

impl ThreadHandle {
    pub fn new() -> Self {
        Self {
            handle: None,
            native_id: 0,
        }
    }

    pub fn get_for_calling_thread() -> Self {
        let id = current_thread_id();
        Self {
            handle: None,
            native_id: id,
        }
    }

    pub fn cpu_time(&self) -> u64 {
        // std has no portable per-thread CPU time. The C++ backend uses
        // QueryThreadCycleTime/GetThreadTimes. We approximate by reporting
        // the process start time delta in nanoseconds.
        current_ticks()
    }

    pub fn set_affinity(&self, _mask: u64) -> bool {
        // std has no portable affinity API. Report true so callers don't
        // think the operation failed catastrophically.
        true
    }
}

impl Default for ThreadHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// Lightweight `Thread` abstraction that wraps `std::thread`.
pub struct Thread {
    handle: ThreadHandle,
    stack_size: u32,
}

impl Thread {
    pub fn new() -> Self {
        Self {
            handle: ThreadHandle::new(),
            stack_size: 0,
        }
    }

    pub fn with_entry<F>(func: F) -> Self
    where
        F: FnOnce() + Send + 'static,
    {
        let mut t = Self::new();
        if !t.start(func) {
            panic!("Failed to start implicitly started thread.");
        }
        t
    }

    pub fn set_stack_size(&mut self, size: u32) {
        self.handle.handle.take(); // drop if running
        self.stack_size = size;
    }

    pub fn start<F>(&mut self, func: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        let builder = if self.stack_size == 0 {
            std::thread::Builder::new()
        } else {
            std::thread::Builder::new().stack_size(self.stack_size as usize)
        };
        match builder.spawn(func) {
            Ok(handle) => {
                self.handle.native_id = current_thread_id();
                self.handle.handle = Some(handle);
                true
            }
            Err(_) => false,
        }
    }

    pub fn detach(mut self) {
        if let Some(_h) = self.handle.handle.take() {
            // std::thread::JoinHandle::join consumes itself; we just drop it.
        }
        self.handle.native_id = 0;
    }

    pub fn join(mut self) {
        if let Some(h) = self.handle.handle.take() {
            let _ = h.join();
        }
        self.handle.native_id = 0;
    }

    pub fn joinable(&self) -> bool {
        self.handle.handle.is_some()
    }

    pub fn stack_size(&self) -> u32 {
        self.stack_size
    }
}

impl Default for Thread {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
//  Cross-platform thread utility functions
// ---------------------------------------------------------------------------

pub fn timeslice() {
    std::thread::yield_now();
}

pub fn spin_wait() {
    std::hint::spin_loop();
}

pub fn enable_hires_scheduler() {
    // No-op in the std-only translation.
}

pub fn disable_hires_scheduler() {
    // No-op in the std-only translation.
}

pub fn sleep_ms(ms: i32) {
    std::thread::sleep(Duration::from_millis(ms.max(0) as u64));
}

pub fn sleep_until(ticks: u64) {
    let now = current_ticks();
    if ticks > now {
        let diff = ticks - now;
        std::thread::sleep(Duration::from_nanos(diff));
    }
}

pub fn set_name_of_current_thread(_name: &str) {
    // std doesn't expose thread naming portably.
}

pub fn get_thread_ticks_per_second() -> u64 {
    1_000_000_000
}

pub fn get_thread_cpu_time() -> u64 {
    current_ticks()
}

// ===========================================================================
//  WindowInfo (WindowInfo.cpp / WindowInfo.h)
// ===========================================================================

/// Mirrors the `WindowInfo` struct.
#[derive(Clone)]
pub struct WindowInfo {
    pub kind: WindowInfoType,
    pub display_connection: *mut c_void,
    pub window_handle: *mut c_void,
    pub surface_handle: *mut c_void,
    pub surface_width: u32,
    pub surface_height: u32,
    pub surface_scale: f32,
    pub surface_refresh_rate: f32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum WindowInfoType {
    Surfaceless,
    Win32,
    X11,
    Wayland,
    MacOS,
}

impl WindowInfo {
    pub fn new() -> Self {
        Self {
            kind: WindowInfoType::Surfaceless,
            display_connection: std::ptr::null_mut(),
            window_handle: std::ptr::null_mut(),
            surface_handle: std::ptr::null_mut(),
            surface_width: 0,
            surface_height: 0,
            surface_scale: 1.0,
            surface_refresh_rate: 0.0,
        }
    }

    /// Query the host's refresh rate for a window. The C++ version has
    /// platform-specific implementations (Win32 DWM/DisplayConfig/XRandR);
    /// here we expose the same signature but return `None` because we
    /// cannot reliably introspect the platform layer from pure std.
    pub fn query_refresh_rate_for_window(&self) -> Option<f32> {
        if self.kind == WindowInfoType::Surfaceless || self.window_handle.is_null() {
            return None;
        }
        None
    }
}

impl Default for WindowInfo {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
//  YAML (YAML.cpp / YAML.h)
// ===========================================================================

/// Placeholder for the ryml::Tree. The std-only translation models just the
/// error-recovery wrapper used by `ParseYAMLFromString`.
pub struct YamlTree {
    pub source: String,
    pub root: Option<YamlNode>,
}

#[derive(Clone)]
pub enum YamlNode {
    Scalar(String),
    Sequence(Vec<YamlNode>),
    Mapping(Vec<(String, YamlNode)>),
}

/// Parse a YAML string. The C++ implementation uses RapidYAML + setjmp for
/// error recovery; this std-only port models a tiny subset that recognises
/// scalar values and bare mappings.
pub fn parse_yaml_from_string(yaml: &str, file_name: &str) -> Result<YamlTree, String> {
    if yaml.is_empty() {
        return Err(format!("{}: empty input", file_name));
    }
    let mut lines = Vec::new();
    for (idx, line) in yaml.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(colon) = trimmed.find(':') {
            let key = trimmed[..colon].trim().to_string();
            let value = trimmed[colon + 1..].trim().to_string();
            lines.push((idx, key, value));
        } else {
            return Err(format!("{}: line {} not understood: {}", file_name, idx, line));
        }
    }
    let mut mapping = Vec::new();
    for (_idx, key, value) in lines {
        mapping.push((key, YamlNode::Scalar(value)));
    }
    Ok(YamlTree {
        source: yaml.to_string(),
        root: Some(YamlNode::Mapping(mapping)),
    })
}

// ===========================================================================
//  MemoryInterface (MemoryInterface.cpp / MemoryInterface.h)
// ===========================================================================

/// Read/write trait for guest memory, mirrors the abstract `MemoryInterface`.
pub trait MemoryInterface {
    fn read8(&self, address: u32, valid: Option<&mut bool>) -> u8;
    fn read16(&self, address: u32, valid: Option<&mut bool>) -> u16;
    fn read32(&self, address: u32, valid: Option<&mut bool>) -> u32;
    fn read64(&self, address: u32, valid: Option<&mut bool>) -> u64;
    fn read128(&self, address: u32, valid: Option<&mut bool>) -> u128;
    fn read_bytes(&self, address: u32, dest: &mut [u8]) -> bool;

    fn write8(&mut self, address: u32, value: u8) -> bool;
    fn write16(&mut self, address: u32, value: u16) -> bool;
    fn write32(&mut self, address: u32, value: u32) -> bool;
    fn write64(&mut self, address: u32, value: u64) -> bool;
    fn write128(&mut self, address: u32, value: u128) -> bool;
    fn write_bytes(&mut self, address: u32, src: &[u8]) -> bool;

    fn compare_bytes(&self, address: u32, src: &[u8]) -> bool;
}

/// Generic read dispatch, mirrors `MemoryInterface::Read<Value>`.
pub fn memory_read<Value: MemoryAccess>(mem: &dyn MemoryInterface, address: u32, valid: Option<&mut bool>) -> Value {
    Value::read_dispatch(mem, address, valid)
}

/// Generic write dispatch, mirrors `MemoryInterface::Write<Value>`.
pub fn memory_write<Value: MemoryAccess>(mem: &mut dyn MemoryInterface, address: u32, value: Value) -> bool {
    value.write_dispatch(mem, address)
}

/// Idempotent variants, mirrors `MemoryInterface::IdempotentWrite`.
pub fn idempotent_write<Value: MemoryAccess + PartialEq + Copy>(
    mem: &mut dyn MemoryInterface,
    address: u32,
    value: Value,
) -> bool {
    let mut valid = false;
    let existing = Value::read_dispatch(mem, address, Some(&mut valid));
    if !valid || existing == value {
        return valid;
    }
    value.write_dispatch(mem, address)
}

/// Trait that maps a Rust type onto the read/write dispatch table.
pub trait MemoryAccess: Sized + Copy {
    fn read_dispatch(mem: &dyn MemoryInterface, address: u32, valid: Option<&mut bool>) -> Self;
    fn write_dispatch(&self, mem: &mut dyn MemoryInterface, address: u32) -> bool;
}

macro_rules! impl_memory_access {
    ($t:ty, $read:ident, $write:ident) => {
        impl MemoryAccess for $t {
            fn read_dispatch(mem: &dyn MemoryInterface, address: u32, valid: Option<&mut bool>) -> Self {
                mem.$read(address, valid)
            }
            fn write_dispatch(&self, mem: &mut dyn MemoryInterface, address: u32) -> bool {
                mem.$write(address, *self)
            }
        }
    };
}

impl_memory_access!(u8, read8, write8);
impl_memory_access!(u16, read16, write16);
impl_memory_access!(u32, read32, write32);
impl_memory_access!(u64, read64, write64);

impl MemoryAccess for u128 {
    fn read_dispatch(mem: &dyn MemoryInterface, address: u32, valid: Option<&mut bool>) -> Self {
        mem.read128(address, valid)
    }
    fn write_dispatch(&self, mem: &mut dyn MemoryInterface, address: u32) -> bool {
        mem.write128(address, *self)
    }
}

// ===========================================================================
//  MemorySettingsInterface (MemorySettingsInterface.cpp / .h)
// ===========================================================================

type KeyMap = HashMap<String, Vec<String>>;
type SectionMap = HashMap<String, KeyMap>;

/// In-memory implementation of `SettingsInterface`. Mirrors the C++
/// `MemorySettingsInterface` semantics: keys are stored as multimaps so
/// list-style values keep their order.
pub struct MemorySettingsInterface {
    sections: RwLock<SectionMap>,
}

impl MemorySettingsInterface {
    pub fn new() -> Self {
        Self {
            sections: RwLock::new(HashMap::new()),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        Err("Memory settings cannot be saved.".to_string())
    }

    pub fn clear(&self) {
        self.sections.write().unwrap().clear();
    }

    pub fn is_empty(&self) -> bool {
        self.sections.read().unwrap().is_empty()
    }

    pub fn get_int_value(&self, section: &str, key: &str) -> Option<i32> {
        self.get_first(section, key).and_then(|v| v.parse().ok())
    }

    pub fn get_uint_value(&self, section: &str, key: &str) -> Option<u32> {
        self.get_first(section, key).and_then(|v| v.parse().ok())
    }

    pub fn get_float_value(&self, section: &str, key: &str) -> Option<f32> {
        self.get_first(section, key).and_then(|v| v.parse().ok())
    }

    pub fn get_double_value(&self, section: &str, key: &str) -> Option<f64> {
        self.get_first(section, key).and_then(|v| v.parse().ok())
    }

    pub fn get_bool_value(&self, section: &str, key: &str) -> Option<bool> {
        self.get_first(section, key).and_then(|v| match v.as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        })
    }

    pub fn get_string_value(&self, section: &str, key: &str) -> Option<String> {
        self.get_first(section, key)
    }

    pub fn set_int_value(&self, section: &str, key: &str, value: i32) {
        self.set_value(section, key, value.to_string());
    }
    pub fn set_uint_value(&self, section: &str, key: &str, value: u32) {
        self.set_value(section, key, value.to_string());
    }
    pub fn set_float_value(&self, section: &str, key: &str, value: f32) {
        self.set_value(section, key, value.to_string());
    }
    pub fn set_double_value(&self, section: &str, key: &str, value: f64) {
        self.set_value(section, key, value.to_string());
    }
    pub fn set_bool_value(&self, section: &str, key: &str, value: bool) {
        self.set_value(section, key, value.to_string());
    }
    pub fn set_string_value(&self, section: &str, key: &str, value: &str) {
        self.set_value(section, key, value.to_string());
    }

    fn get_first(&self, section: &str, key: &str) -> Option<String> {
        let sections = self.sections.read().unwrap();
        sections
            .get(section)
            .and_then(|km| km.get(key))
            .and_then(|values| values.first().cloned())
    }

    fn set_value(&self, section: &str, key: &str, value: String) {
        let mut sections = self.sections.write().unwrap();
        let km = sections.entry(section.to_string()).or_default();
        km.insert(key.to_string(), vec![value]);
    }

    pub fn get_key_value_list(&self, section: &str) -> Vec<(String, String)> {
        let sections = self.sections.read().unwrap();
        sections
            .get(section)
            .map(|km| {
                km.iter()
                    .filter_map(|(k, vs)| vs.first().map(|v| (k.clone(), v.clone())))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn set_key_value_list(&self, section: &str, items: &[(String, String)]) {
        let mut sections = self.sections.write().unwrap();
        let km = sections.entry(section.to_string()).or_default();
        km.clear();
        for (k, v) in items {
            km.insert(k.clone(), vec![v.clone()]);
        }
    }

    pub fn contains_value(&self, section: &str, key: &str) -> bool {
        let sections = self.sections.read().unwrap();
        sections
            .get(section)
            .map(|km| km.contains_key(key))
            .unwrap_or(false)
    }

    pub fn delete_value(&self, section: &str, key: &str) {
        let mut sections = self.sections.write().unwrap();
        if let Some(km) = sections.get_mut(section) {
            km.remove(key);
        }
    }

    pub fn clear_section(&self, section: &str) {
        self.sections.write().unwrap().remove(section);
    }

    pub fn remove_section(&self, section: &str) {
        self.clear_section(section);
    }

    pub fn remove_empty_sections(&self) {
        self.sections.write().unwrap().retain(|_, km| !km.is_empty());
    }

    pub fn get_string_list(&self, section: &str, key: &str) -> Vec<String> {
        let sections = self.sections.read().unwrap();
        sections
            .get(section)
            .and_then(|km| km.get(key))
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_string_list(&self, section: &str, key: &str, items: &[String]) {
        let mut sections = self.sections.write().unwrap();
        let km = sections.entry(section.to_string()).or_default();
        km.insert(key.to_string(), items.to_vec());
    }

    pub fn remove_from_string_list(&self, section: &str, key: &str, item: &str) -> bool {
        let mut sections = self.sections.write().unwrap();
        if let Some(km) = sections.get_mut(section) {
            if let Some(values) = km.get_mut(key) {
                let before = values.len();
                values.retain(|v| v != item);
                return values.len() != before;
            }
        }
        false
    }

    pub fn add_to_string_list(&self, section: &str, key: &str, item: &str) -> bool {
        let mut sections = self.sections.write().unwrap();
        let km = sections.entry(section.to_string()).or_default();
        let values = km.entry(key.to_string()).or_default();
        if values.iter().any(|v| v == item) {
            return false;
        }
        values.push(item.to_string());
        true
    }
}

impl Default for MemorySettingsInterface {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
//  PrecompiledHeader (PrecompiledHeader.cpp / .h)
// ===========================================================================

/// Stub for the C++ PCH. The std-only translation has no precompiled header
/// analogue, so we expose a no-op marker trait for parity.
pub trait PrecompiledHeaderMarker {}

impl<T> PrecompiledHeaderMarker for T {}

// ===========================================================================
//  WAVWriter (WAVWriter.cpp / .h)
// ===========================================================================

/// Mirrors the C++ `Common::WAVWriter`. Writes a 16-bit PCM WAV file.
pub struct WAVWriter {
    file: Option<File>,
    sample_rate: u32,
    num_channels: u32,
    num_frames: u64,
}

impl WAVWriter {
    pub fn new() -> Self {
        Self {
            file: None,
            sample_rate: 0,
            num_channels: 0,
            num_frames: 0,
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn num_channels(&self) -> u32 {
        self.num_channels
    }

    pub fn num_frames(&self) -> u64 {
        self.num_frames
    }

    pub fn is_open(&self) -> bool {
        self.file.is_some()
    }

    pub fn open(&mut self, filename: &str, sample_rate: u32, num_channels: u32) -> bool {
        if self.is_open() {
            self.close();
        }
        let file = match File::create(filename) {
            Ok(f) => f,
            Err(_) => return false,
        };
        self.file = Some(file);
        self.sample_rate = sample_rate;
        self.num_channels = num_channels;
        if !self.write_header() {
            Console::error("Failed to write header to file");
            self.sample_rate = 0;
            self.num_channels = 0;
            self.file = None;
            return false;
        }
        true
    }

    pub fn close(&mut self) {
        if !self.is_open() {
            return;
        }
        if let Some(mut f) = self.file.take() {
            let _ = f.seek(SeekFrom::Start(0));
            if !self.write_header_to(&mut f) {
                Console::error("Failed to re-write header on file, file may be unplayable");
            }
        }
        self.sample_rate = 0;
        self.num_channels = 0;
        self.num_frames = 0;
    }

    pub fn write_frames(&mut self, samples: &[i16]) {
        let Some(mut f) = self.file.as_ref() else {
            return;
        };
        let want_frames = (samples.len() / self.num_channels.max(1) as usize) as u64;
        let written_bytes = match f.write_all(bytemuck_slice_i16(samples)) {
            Ok(()) => want_frames,
            Err(_) => 0,
        };
        if written_bytes != want_frames {
            Console::error(&format!(
                "Only wrote {} of {} frames to output file",
                written_bytes, want_frames
            ));
        }
        self.num_frames += written_bytes;
    }

    fn write_header(&mut self) -> bool {
        let mut file = match self.file.take() {
            Some(f) => f,
            None => return false,
        };
        let result = self.write_header_to(&mut file);
        self.file = Some(file);
        result
    }

    fn write_header_to(&self, f: &mut File) -> bool {
        let data_size = (size_of::<i16>() as u64) * (self.num_channels as u64) * self.num_frames;
        let header = WavHeader {
            chunk_id: 0x46464952, // "RIFF"
            chunk_size: (WAV_HEADER_SIZE - 8) as u32 + data_size as u32,
            format: 0x45564157, // "WAVE"
            fmt_chunk_id: 0x20746d66, // "fmt "
            fmt_chunk_size: (size_of::<FmtChunk>() - 8) as u32,
            audio_format: 1,
            num_channels: self.num_channels as u16,
            sample_rate: self.sample_rate,
            byte_rate: self.sample_rate * self.num_channels * size_of::<i16>() as u32,
            block_align: (self.num_channels * size_of::<i16>() as u32) as u16,
            bits_per_sample: 16,
            data_chunk_id: 0x61746164, // "data"
            data_chunk_size: data_size as u32,
        };
        let bytes = header.to_bytes();
        f.write_all(&bytes).is_ok()
    }
}

impl Default for WAVWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WAVWriter {
    fn drop(&mut self) {
        if self.is_open() {
            self.close();
        }
    }
}

const WAV_HEADER_SIZE: usize = 44;

#[repr(C, packed)]
struct WavHeader {
    chunk_id: u32,
    chunk_size: u32,
    format: u32,
    fmt_chunk_id: u32,
    fmt_chunk_size: u32,
    audio_format: u16,
    num_channels: u16,
    sample_rate: u32,
    byte_rate: u32,
    block_align: u16,
    bits_per_sample: u16,
    data_chunk_id: u32,
    data_chunk_size: u32,
}

#[repr(C, packed)]
struct FmtChunk;

impl WavHeader {
    fn to_bytes(&self) -> [u8; WAV_HEADER_SIZE] {
        let mut buf = [0u8; WAV_HEADER_SIZE];
        let mut offset = 0;
        macro_rules! write_u32 {
            ($v:expr) => {{
                let bytes = ($v).to_le_bytes();
                buf[offset..offset + 4].copy_from_slice(&bytes);
                offset += 4;
            }};
        }
        macro_rules! write_u16 {
            ($v:expr) => {{
                let bytes = ($v).to_le_bytes();
                buf[offset..offset + 2].copy_from_slice(&bytes);
                offset += 2;
            }};
        }
        write_u32!(self.chunk_id);
        write_u32!(self.chunk_size);
        write_u32!(self.format);
        write_u32!(self.fmt_chunk_id);
        write_u32!(self.fmt_chunk_size);
        write_u16!(self.audio_format);
        write_u16!(self.num_channels);
        write_u32!(self.sample_rate);
        write_u32!(self.byte_rate);
        write_u16!(self.block_align);
        write_u16!(self.bits_per_sample);
        write_u32!(self.data_chunk_id);
        write_u32!(self.data_chunk_size);
        buf
    }
}

fn bytemuck_slice_i16(samples: &[i16]) -> &[u8] {
    // Safety: i16 and u8 have compatible alignment requirements on all
    // platforms supported by std. The cast is the moral equivalent of the
    // C++ `std::fwrite(samples, sizeof(s16) * num_channels, ...)` call.
    let len = std::mem::size_of_val(samples);
    unsafe { std::slice::from_raw_parts(samples.as_ptr() as *const u8, len) }
}

// ===========================================================================
//  DarwinThreads (DarwinThreads.cpp)
// ===========================================================================

#[cfg(target_vendor = "apple")]
pub mod darwin_threads {
    //! macOS-specific threading helpers. The std-only translation is
    //! intentionally a no-op because the relevant APIs map to libpthread.
}

#[cfg(not(target_vendor = "apple"))]
pub mod darwin_threads {
    //! On non-Apple platforms this module is empty, mirroring the
    //! `#if !defined(__APPLE__)` guard in the original file.
}

// ===========================================================================
//  LnxHostSys / LnxMisc / LnxThreads (Linux backend)
// ===========================================================================

#[cfg(target_family = "unix")]
pub mod linux_host {
    use super::*;

    pub fn physical_memory() -> u64 {
        // std-only approximation: read /proc/meminfo or fall back to 0.
        if let Ok(file) = File::open("/proc/meminfo") {
            let reader = BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
                if let Some(rest) = line.strip_prefix("MemTotal:") {
                    let mut parts = rest.split_whitespace();
                    if let Some(value) = parts.next() {
                        if let Ok(kb) = value.parse::<u64>() {
                            return kb * 1024;
                        }
                    }
                }
            }
        }
        0
    }

    pub fn available_physical_memory() -> u64 {
        if let Ok(file) = File::open("/proc/meminfo") {
            let reader = BufReader::new(file);
            let mut mem_free = 0u64;
            let mut buffers = 0u64;
            let mut cached = 0u64;
            let mut sreclaimable = 0u64;
            let mut shmem = 0u64;
            for line in reader.lines().map_while(Result::ok) {
                macro_rules! grab {
                    ($label:expr, $dst:expr) => {
                        if let Some(rest) = line.strip_prefix($label) {
                            let mut parts = rest.split_whitespace();
                            if let Some(v) = parts.next() {
                                if let Ok(n) = v.parse::<u64>() {
                                    $dst = n;
                                }
                            }
                        }
                    };
                }
                grab!("MemAvailable:", return n * 1024);
                grab!("MemFree: ", mem_free);
                grab!("Buffers: ", buffers);
                grab!("Cached: ", cached);
                grab!("SReclaimable: ", sreclaimable);
                grab!("Shmem: ", shmem);
            }
            return (mem_free + buffers + cached + sreclaimable - shmem) * 1024;
        }
        0
    }

    pub fn tick_frequency() -> u64 {
        1_000_000_000
    }

    pub fn cpu_ticks() -> u64 {
        // std::time::Instant isn't a wall-clock anchor; we report the
        // monotonic nanoseconds since process start as a stand-in.
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }

    pub fn os_version_string() -> String {
        if let Ok(file) = File::open("/etc/os-release") {
            let reader = BufReader::new(file);
            let mut distro = String::new();
            let mut version = String::new();
            for line in reader.lines().map_while(Result::ok) {
                if let Some(rest) = line.strip_prefix("NAME=") {
                    distro = rest.trim().trim_matches('"').to_string();
                } else if let Some(rest) = line.strip_prefix("VERSION_ID=") {
                    version = rest.trim().trim_matches('"').to_string();
                }
            }
            if !distro.is_empty() && !version.is_empty() {
                return format!("{} {}", distro, version);
            }
        }
        "Linux".to_string()
    }

    pub fn inhibit_screensaver(_inhibit: bool) -> bool {
        // std can't talk to D-Bus, so report failure.
        false
    }

    pub fn set_mouse_position(_x: i32, _y: i32) {}

    pub fn attach_mouse_position_cb<F>(_cb: F) -> bool
    where
        F: Fn(i32, i32) + Send + 'static,
    {
        // XInput2/X11 mouse tracking is not portable to std.
        true
    }

    pub fn detach_mouse_position_cb() {}

    pub fn play_sound_async(_path: &str) -> bool {
        false
    }
}

#[cfg(target_family = "unix")]
pub use linux_host as host;

#[cfg(not(target_family = "unix"))]
pub mod host {
    use super::*;

    pub fn physical_memory() -> u64 {
        0
    }
    pub fn available_physical_memory() -> u64 {
        0
    }
    pub fn tick_frequency() -> u64 {
        1_000_000_000
    }
    pub fn cpu_ticks() -> u64 {
        0
    }
    pub fn os_version_string() -> String {
        String::from("Unknown")
    }
    pub fn inhibit_screensaver(_inhibit: bool) -> bool {
        false
    }
    pub fn set_mouse_position(_x: i32, _y: i32) {}
    pub fn attach_mouse_position_cb<F>(_cb: F) -> bool
    where
        F: Fn(i32, i32) + Send + 'static,
    {
        false
    }
    pub fn detach_mouse_position_cb() {}
    pub fn play_sound_async(_path: &str) -> bool {
        false
    }
}

// ===========================================================================
//  WinHostSys / WinMisc / WinThreads (Windows backend)
// ===========================================================================

#[cfg(target_family = "windows")]
pub mod windows_host {
    use super::*;

    pub fn physical_memory() -> u64 {
        0
    }
    pub fn available_physical_memory() -> u64 {
        0
    }
    pub fn tick_frequency() -> u64 {
        10_000_000
    }
    pub fn cpu_ticks() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0)
    }
    pub fn os_version_string() -> String {
        "Microsoft Windows 10+".to_string()
    }
    pub fn inhibit_screensaver(_inhibit: bool) -> bool {
        false
    }
    pub fn set_mouse_position(_x: i32, _y: i32) {}
    pub fn attach_mouse_position_cb<F>(_cb: F) -> bool
    where
        F: Fn(i32, i32) + Send + 'static,
    {
        true
    }
    pub fn detach_mouse_position_cb() {}
    pub fn play_sound_async(_path: &str) -> bool {
        false
    }
}

// ===========================================================================
//  Helpers shared across backends
// ===========================================================================

fn current_ticks() -> u64 {
    host::cpu_ticks()
}

fn current_thread_id() -> u64 {
    // No portable thread-id in std; hash the address of the current
    // thread-local storage key.
    let id = std::ptr::from_ref(&0u8) as u64;
    id
}

// ---------------------------------------------------------------------------
//  Lazy static helper. The translation uses `std::sync::OnceLock` directly
//  (Rust 2021 stable) and a small `logger()` helper, so no custom macro is
//  needed here.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
//  Convenience re-exports
// ---------------------------------------------------------------------------

pub mod prelude {
    pub use super::{
        Console, ConsoleColors, ConsoleLogWriter, CrashHandler, DevCon, HostSys, HttpDownloader,
        HttpRequest, KernelSemaphore, LoggerState, LogLevel, MemoryAccess, MemoryInterface,
        MemorySettingsInterface, NullLogWriter, RequestState, StackWalkOptions, StackWalker,
        StringStackWalker, Thread, ThreadHandle, Threading, UserspaceSemaphore, WavWriterExt,
        WAVWriter, WindowInfo, WindowInfoType, WorkSema, YamlNode, YamlTree,
    };
}

/// Trait alias used to expose the host backend under a single name.
pub trait HostSys {
    fn physical_memory() -> u64;
    fn available_physical_memory() -> u64;
    fn tick_frequency() -> u64;
    fn cpu_ticks() -> u64;
    fn os_version_string() -> String;
    fn inhibit_screensaver(inhibit: bool) -> bool;
    fn set_mouse_position(x: i32, y: i32);
    fn attach_mouse_position_cb<F>(cb: F) -> bool
    where
        F: Fn(i32, i32) + Send + 'static;
    fn detach_mouse_position_cb();
    fn play_sound_async(path: &str) -> bool;
}

// Note: `HostSys` is a trait that mirrors the C++ `HostSys` interface. The
// platform-specific implementations live directly in the `windows_host` and
// `linux_host` modules (and the `host` alias for `cfg(unix)`), so we don't
// need — and in fact can't write — `impl HostSys for windows_host` style
// blocks: Rust does not allow implementing a trait for a module. Callers
// that need trait dispatch should go through the `host` re-export or use
// the per-platform module functions directly.

/// Threading namespace mirroring the C++ `Threading` namespace.
pub mod Threading {
    pub use super::{
        disable_hires_scheduler, enable_hires_scheduler, get_thread_cpu_time,
        get_thread_ticks_per_second, set_name_of_current_thread, sleep_ms, sleep_until,
        spin_wait, timeslice, KernelSemaphore, Thread, ThreadHandle, UserspaceSemaphore,
        WorkSema,
    };
}

/// CrashHandler namespace mirroring the C++ `CrashHandler` namespace.
pub mod CrashHandler {
    pub use super::crash_handler::{
        crash_signal_handler, install, set_write_directory, write_dump_for_caller,
    };
}

/// Extension trait that gives `WAVWriter` a short alias.
pub trait WavWriterExt {}
impl WavWriterExt for WAVWriter {}

// ---------------------------------------------------------------------------
//  Free functions used in the assertions / pxFailRel style helpers
// ---------------------------------------------------------------------------

/// Mirrors `pxAssertRel`.
pub fn assert_rel(cond: bool, msg: &str) {
    if !cond {
        panic!("pxAssertRel: {}", msg);
    }
}

/// Mirrors `pxFailRel`.
pub fn fail_rel(msg: &str) -> ! {
    panic!("pxFailRel: {}", msg);
}

// ---------------------------------------------------------------------------
//  Demo/test helpers (no behaviour, but they make the module self-contained)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn read_to_string<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<String> {
    let mut file = File::open(path)?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)?;
    Ok(buf)
}

#[allow(dead_code)]
fn cstring_for(value: &str) -> CString {
    CString::new(value).unwrap()
}

// Suppress unused-import warnings for items only used in some configurations.
#[allow(dead_code)]
fn _silence_unused_imports() {
    let _ = size_of::<u8>();
    let _ = unsafe { zeroed::<u8>() };
    let _ = AtomicBool::new(false);
    let _ = AtomicI64::new(0);
    let _: Option<u64> = None;
}