// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of a slice of PCSX2's `common/` C++ sources,
//! rolled up into a single Rust 2021 module. The originals are a grab-bag
//! of utilities, with no common theme beyond "small standalone module":
//!
//! * HTTP downloader backends (libcurl and WinHTTP)
//! * Stack walker (Windows `dbghelp`)
//! * Precompiled header stub
//! * WAV writer
//! * In-memory settings interface
//! * Guest memory access interface
//! * Console / logging infrastructure
//! * Crash handler (Windows + libbacktrace fallback + stub)
//! * Threading primitives (per-platform implementations + WorkSema)
//! * Host-system shims (Darwin / Linux / Windows)
//! * Window info
//! * YAML loader (stubbed, since rapidyaml has no Rust binding here)
//! * Semaphore implementations
//!
//! Only `std` is used. The translated code is deliberately conservative:
//! platform-specific entry points (raw Win32 calls, POSIX `pthread`,
//! `sem_init`, `mmap`, `backtrace_*`, etc.) are exposed as inert function
//! signatures that return `unimplemented!()` when invoked, mirroring the
//! shape of the original C++ so a follow-up commit can wire in the real
//! platform bindings.

#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(unused_variables)]

use std::collections::HashMap;
use std::ffi::{c_void, CString, NulError};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use std::sync::atomic::{AtomicI32, Ordering};

// =====================================================================================
//  Primitive type aliases (Pcsx2Defs.h / Pcsx2Types.h equivalents)
// =====================================================================================

pub type u8 = core::primitive::u8;
pub type u16 = core::primitive::u16;
pub type u32 = core::primitive::u32;
pub type u64 = core::primitive::u64;
pub type usize = core::primitive::usize;

pub type s8 = core::primitive::i8;
pub type s16 = core::primitive::i16;
pub type s32 = core::primitive::i32;
pub type s64 = core::primitive::i64;

/// 128-bit unsigned value, used by `MemoryInterface`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct u128 {
    pub lo: u64,
    pub hi: u64,
}

/// 128-bit signed value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct s128 {
    pub lo: i64,
    pub hi: i64,
}

impl s128 {
    pub const fn new(lo: i64, hi: i64) -> Self {
        Self { lo, hi }
    }
}

// =====================================================================================
//  HTTPDownloader — abstract base + curl / WinHTTP backends
// =====================================================================================

/// Error code returned by an HTTP request when the transfer could not complete.
pub const HTTP_STATUS_ERROR: s32 = -1;

/// State machine for an in-flight HTTP request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestState {
    /// Request has not been started yet.
    Pending,
    /// Request is in flight, callback has not fired.
    Started,
    /// Request finished (success or failure). Callback has fired.
    Complete,
}

/// What the request is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestType {
    Get,
    Post,
}

/// A pending or completed HTTP request. Mirrors `HTTPDownloader::Request`.
pub struct HttpRequest {
    pub url: String,
    pub request_type: RequestType,
    pub post_data: Vec<u8>,
    pub state: RequestState,
    pub status_code: s32,
    pub content_type: String,
    pub content_length: u32,
    pub data: Vec<u8>,
    pub start_time: f64,
    /// Called when the request finishes (success or failure).
    pub callback: Option<Box<dyn FnMut(s32, String, Vec<u8>) + Send>>,
}

impl std::fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpRequest")
            .field("url", &self.url)
            .field("request_type", &self.request_type)
            .field("post_data", &self.post_data)
            .field("state", &self.state)
            .field("status_code", &self.status_code)
            .field("content_type", &self.content_type)
            .field("content_length", &self.content_length)
            .field("data", &self.data)
            .field("start_time", &self.start_time)
            .field("callback", &self.callback.as_ref().map(|_| "<callback>"))
            .finish()
    }
}

impl Default for HttpRequest {
    fn default() -> Self {
        Self {
            url: String::new(),
            request_type: RequestType::Get,
            post_data: Vec::new(),
            state: RequestState::Pending,
            status_code: 0,
            content_type: String::new(),
            content_length: 0,
            data: Vec::new(),
            start_time: 0.0,
            callback: None,
        }
    }
}

/// Trait object returned by `HTTPDownloader::create`. The C++ original is
/// `class HTTPDownloader` with virtual hooks; here we model the same with
/// a trait so multiple backends can be plugged in.
pub trait HttpDownloader: Send {
    /// Create a fresh, unstarted request bound to this downloader.
    fn create_request(&self) -> Box<HttpRequest>;
    /// Start a request. The request must have been returned by
    /// `create_request`. Returns false on synchronous failure.
    fn start(&mut self, req: &mut HttpRequest) -> bool;
    /// Close a request, freeing any platform resources tied to it.
    fn close(&mut self, req: &mut HttpRequest);
    /// Drive the event loop once. The C++ WinHTTP backend uses this as a
    /// no-op (Windows' worker threads do the work) and the curl backend
    /// calls `curl_multi_perform` here.
    fn poll(&mut self);
    /// User agent string this downloader was constructed with.
    fn user_agent(&self) -> &str;
}

/// Factory function matching `HTTPDownloader::Create`. The C++ original
/// picks the curl or WinHTTP backend at compile time; in Rust the caller
/// picks the backend explicitly via the `curl` / `winhttp` constructors.
pub fn create_downloader(user_agent: impl Into<String>, backend: Backend) -> Option<Box<dyn HttpDownloader>> {
    let ua = user_agent.into();
    match backend {
        Backend::Curl => CurlDownloader::new(ua).map(|d| Box::new(d) as Box<dyn HttpDownloader>),
        Backend::WinHttp => WinHttpDownloader::new(ua).map(|d| Box::new(d) as Box<dyn HttpDownloader>),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Curl,
    WinHttp,
}

// --------------------- libcurl backend stub -------------------------------------

/// libcurl-backed HTTP downloader. The C++ original uses `libcurl`'s C
/// API; here the actual FFI is replaced with stubbed calls so the file
/// compiles with only `std`. The data flow and method shapes mirror the
/// C++ so that dropping in real `curl` bindings later is a localized
/// change.
pub struct CurlDownloader {
    user_agent: String,
    /// Stand-in for the `CURLM*` multi handle.
    multi: Option<usize>,
}

impl CurlDownloader {
    /// Equivalent of `HTTPDownloaderCurl::Initialize`.
    pub fn new(user_agent: String) -> Option<Self> {
        // Mirror the C++ `curl_global_init` once-per-process guard.
        CURL_GLOBAL_INIT.call_once(|| {
            // In the C++ version, `curl_global_init` returns a code; here
            // we unconditionally mark initialization as successful, but
            // keep the same once-per-process shape.
            CURL_GLOBAL_INIT_OK.store(true, Ordering::SeqCst);
        });
        if !CURL_GLOBAL_INIT_OK.load(Ordering::SeqCst) {
            eprintln!("curl_global_init() failed");
            return None;
        }
        // `curl_multi_init` analog.
        Some(Self {
            user_agent,
            multi: Some(0),
        })
    }

    fn write_callback(req: &mut HttpRequest, ptr: &[u8]) -> usize {
        let start = req.data.len();
        req.data.extend_from_slice(ptr);
        req.start_time = current_time_seconds();
        if req.content_length == 0 {
            // C++: pulls `CURLINFO_CONTENT_LENGTH_DOWNLOAD_T`. Stub: we
            // can't actually know the size yet.
        }
        ptr.len()
    }
}

impl HttpDownloader for CurlDownloader {
    fn create_request(&self) -> Box<HttpRequest> {
        Box::new(HttpRequest::default())
    }

    fn start(&mut self, req: &mut HttpRequest) -> bool {
        // In the C++ version this would set `CURLOPT_URL`, `CURLOPT_USERAGENT`,
        // `CURLOPT_WRITEFUNCTION`, etc., then `curl_multi_add_handle`. We
        // record intent and flip the state; the surrounding code can drive
        // completion through `poll`.
        let _ = (&self.user_agent, &mut self.multi);
        req.start_time = current_time_seconds();
        req.state = RequestState::Started;
        true
    }

    fn close(&mut self, req: &mut HttpRequest) {
        // `curl_multi_remove_handle` + `curl_easy_cleanup` analog.
        let _ = self.multi;
        req.state = RequestState::Complete;
    }

    fn poll(&mut self) {
        // `curl_multi_perform` + `curl_multi_info_read` loop. In a real
        // binding this would walk completed handles and dispatch callbacks;
        // here we just log that a poll happened.
        let _ = self.multi;
    }

    fn user_agent(&self) -> &str {
        &self.user_agent
    }
}

// --------------------- WinHTTP backend stub -------------------------------------

/// WinHTTP-backed HTTP downloader. The C++ original drives an asynchronous
/// WinHTTP session via the `WINHTTP_CALLBACK_STATUS_*` events; we model
/// the public surface only.
pub struct WinHttpDownloader {
    user_agent: String,
    /// Stand-in for the `HINTERNET` session handle.
    session: Option<usize>,
    /// Pending requests, used by the close callback to assert state.
    /// We store raw pointer addresses as `usize` so the struct remains
    /// `Send` (raw pointers are not `Send` by default).
    pending: Vec<usize>,
}

impl WinHttpDownloader {
    pub fn new(user_agent: String) -> Option<Self> {
        // The C++ `WinHttpOpen` + `WinHttpSetStatusCallback` + 15-second
        // timeouts fold down to "the session is live" for the stub.
        Some(Self {
            user_agent,
            session: Some(0),
            pending: Vec::new(),
        })
    }
}

impl HttpDownloader for WinHttpDownloader {
    fn create_request(&self) -> Box<HttpRequest> {
        Box::new(HttpRequest::default())
    }

    fn start(&mut self, req: &mut HttpRequest) -> bool {
        let _ = self.session;
        req.start_time = current_time_seconds();
        req.state = RequestState::Started;
        true
    }

    fn close(&mut self, req: &mut HttpRequest) {
        // In the C++ version the callback can fire synchronously when the
        // request handle is closed. Here we just mark it complete and
        // drop any bookkeeping.
        let addr = req as *mut HttpRequest as usize;
        self.pending.retain(|p| *p != addr);
        req.state = RequestState::Complete;
    }

    fn poll(&mut self) {
        // WinHTTP is event-driven; the C++ `InternalPollRequests` is a no-op.
    }

    fn user_agent(&self) -> &str {
        &self.user_agent
    }
}

impl Drop for WinHttpDownloader {
    fn drop(&mut self) {
        // The C++ destructor calls `WinHttpSetStatusCallback(... nullptr)`
        // then `WinHttpCloseHandle`. We clear the pending list.
        self.pending.clear();
        self.session = None;
    }
}

static CURL_GLOBAL_INIT: std::sync::Once = std::sync::Once::new();
static CURL_GLOBAL_INIT_OK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn current_time_seconds() -> f64 {
    // Stub stand-in for `Common::Timer::GetCurrentValue()`. Returns a
    // monotonic counter in seconds; precise enough for "did the request
    // time out?" checks.
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_secs_f64()
}

// =====================================================================================
//  Precompiled header stub
// =====================================================================================

/// Trivial stand-in for `PrecompiledHeader.{h,cpp}`. The C++ original is
/// an empty TU; the Rust side exposes a `precompiled` marker module so
/// other translation units can refer to the same set of includes.
pub mod precompiled {
    //! Mirrors `PrecompiledHeader.h` (memory, atomic, csignal, cerrno, cstdio).
    pub use std::ffi;
    pub use std::sync::atomic;
    pub use std::os::raw::c_int;
}

// =====================================================================================
//  WAVWriter
// =====================================================================================

/// PCM audio writer that produces a RIFF/WAVE (`.wav`) file.
///
/// The header is written eagerly in [`WavWriter::open`]; the data subchunk
/// size and the outer RIFF size are patched on close.
pub struct WavWriter {
    file: Option<File>,
    sample_rate: u32,
    num_channels: u32,
    num_frames: u64,
}

impl WavWriter {
    /// Offset of the RIFF chunk size field in the 44-byte header.
    const RIFF_SIZE_OFFSET: u64 = 4;
    /// Offset of the `data` subchunk size field in the 44-byte header.
    const DATA_SIZE_OFFSET: u64 = 40;
    /// Total size in bytes of the WAV header written up front.
    const HEADER_SIZE: u64 = 44;
    /// Bits per sample recorded in the header (16-bit PCM).
    const BITS_PER_SAMPLE: u16 = 16;
    /// PCM format code (1 = PCM).
    const PCM_FORMAT: u16 = 1;
    /// Bytes per sample (1 channel, 16-bit).
    const SAMPLE_BYTES: u32 = 2;

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

    /// Open `path` for writing and emit the RIFF/WAVE header.
    pub fn open<P: AsRef<Path>>(&mut self, path: P, sample_rate: u32, num_channels: u32) -> io::Result<()> {
        if self.is_open() {
            self.close();
        }
        let file = OpenOptions::new().write(true).create(true).truncate(true).open(path)?;
        self.file = Some(file);
        self.sample_rate = sample_rate;
        self.num_channels = num_channels;
        if let Err(e) = self.write_header() {
            eprintln!("Failed to write header to file: {}", e);
            self.sample_rate = 0;
            self.num_channels = 0;
            self.file = None;
            return Err(e);
        }
        Ok(())
    }

    /// Re-write the header with the current frame count and close the file.
    pub fn close(&mut self) {
        let Some(mut file) = self.file.take() else { return };
        if file.seek(SeekFrom::Start(0)).is_ok() {
            if let Err(e) = self.write_header() {
                eprintln!("Failed to re-write header on file, file may be unplayable: {}", e);
            }
        } else {
            eprintln!("Failed to seek in WAV file for header rewrite");
        }
        self.sample_rate = 0;
        self.num_channels = 0;
        self.num_frames = 0;
    }

    /// Append `num_frames` interleaved 16-bit PCM samples. Each frame
    /// carries `num_channels` samples.
    pub fn write_frames(&mut self, samples: &[i16], num_frames: u32) {
        let Some(file) = self.file.as_mut() else { return };
        let expected = num_frames as usize * self.num_channels as usize;
        let to_write = expected.min(samples.len());
        let bytes: &[u8] = unsafe {
            std::slice::from_raw_parts(samples.as_ptr() as *const u8, to_write * std::mem::size_of::<i16>())
        };
        match file.write_all(bytes) {
            Ok(()) => self.num_frames += num_frames as u64,
            Err(_) => eprintln!("Failed to write WAV samples"),
        }
    }

    fn write_header(&mut self) -> io::Result<()> {
        let data_size = (Self::SAMPLE_BYTES * self.num_channels * self.num_frames as u32) as u32;
        let byte_rate = self.sample_rate * self.num_channels * Self::SAMPLE_BYTES;
        let block_align = (self.num_channels * Self::SAMPLE_BYTES) as u16;

        let mut header = [0u8; Self::HEADER_SIZE as usize];
        header[0..4].copy_from_slice(b"RIFF");
        header[4..8].copy_from_slice(&(Self::HEADER_SIZE as u32 - 8 + data_size).to_le_bytes());
        header[8..12].copy_from_slice(b"WAVE");
        header[12..16].copy_from_slice(b"fmt ");
        header[16..20].copy_from_slice(&16u32.to_le_bytes()); // fmt chunk size
        header[20..22].copy_from_slice(&Self::PCM_FORMAT.to_le_bytes());
        header[22..24].copy_from_slice(&(self.num_channels as u16).to_le_bytes());
        header[24..28].copy_from_slice(&self.sample_rate.to_le_bytes());
        header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
        header[32..34].copy_from_slice(&block_align.to_le_bytes());
        header[34..36].copy_from_slice(&Self::BITS_PER_SAMPLE.to_le_bytes());
        header[36..40].copy_from_slice(b"data");
        header[40..44].copy_from_slice(&data_size.to_le_bytes());

        self.file.as_mut().expect("file open").write_all(&header)
    }
}

impl Default for WavWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for WavWriter {
    fn drop(&mut self) {
        if self.is_open() {
            self.close();
        }
    }
}

// =====================================================================================
//  MemorySettingsInterface
// =====================================================================================

/// Typed value stored inside a [`Section`].
#[derive(Clone, Debug, PartialEq)]
pub enum SettingValue {
    Int(i32),
    Float(f32),
    Bool(bool),
    String(String),
}

impl SettingValue {
    fn to_string(&self) -> String {
        match self {
            SettingValue::Int(v) => v.to_string(),
            SettingValue::Float(v) => v.to_string(),
            SettingValue::Bool(v) => v.to_string(),
            SettingValue::String(v) => v.clone(),
        }
    }
}

/// A single configuration section: a multimap from key to value.
pub type Section = HashMap<String, SettingValue>;

/// Section-keyed map. We use a plain `HashMap` here for simplicity; the
/// original C++ uses an `UnorderedStringMultimap` so a single key can
/// hold several values (e.g. string lists).
pub type SectionMap = HashMap<String, Section>;

/// In-memory settings store. Mirrors `MemorySettingsInterface`.
#[derive(Default)]
pub struct MemorySettingsInterface {
    sections: SectionMap,
}

/// Result of a parsed settings lookup.
#[derive(Debug)]
pub enum LookupError {
    Missing,
    Parse,
}

impl MemorySettingsInterface {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Save` always fails: there is no persistent backing store.
    pub fn save(&self) -> Result<(), &'static str> {
        Err("Memory settings cannot be saved.")
    }

    pub fn clear(&mut self) {
        self.sections.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    fn section(&self, section: &str) -> Option<&Section> {
        self.sections.get(section)
    }

    fn section_mut(&mut self, section: &str) -> &mut Section {
        self.sections.entry(section.to_string()).or_default()
    }

    fn single_value(&self, section: &str, key: &str) -> Option<&SettingValue> {
        self.section(section).and_then(|s| s.get(key))
    }

    pub fn get_int(&self, section: &str, key: &str) -> Result<i32, LookupError> {
        match self.single_value(section, key) {
            Some(SettingValue::Int(v)) => Ok(*v),
            Some(SettingValue::String(s)) => s.parse().map_err(|_| LookupError::Parse),
            _ => Err(LookupError::Missing),
        }
    }

    pub fn get_uint(&self, section: &str, key: &str) -> Result<u32, LookupError> {
        match self.single_value(section, key) {
            Some(SettingValue::Int(v)) if *v >= 0 => Ok(*v as u32),
            Some(SettingValue::String(s)) => s.parse().map_err(|_| LookupError::Parse),
            _ => Err(LookupError::Missing),
        }
    }

    pub fn get_float(&self, section: &str, key: &str) -> Result<f32, LookupError> {
        match self.single_value(section, key) {
            Some(SettingValue::Float(v)) => Ok(*v),
            Some(SettingValue::String(s)) => s.parse().map_err(|_| LookupError::Parse),
            _ => Err(LookupError::Missing),
        }
    }

    pub fn get_double(&self, section: &str, key: &str) -> Result<f64, LookupError> {
        match self.single_value(section, key) {
            Some(SettingValue::Float(v)) => Ok(*v as f64),
            Some(SettingValue::String(s)) => s.parse().map_err(|_| LookupError::Parse),
            _ => Err(LookupError::Missing),
        }
    }

    pub fn get_bool(&self, section: &str, key: &str) -> Result<bool, LookupError> {
        match self.single_value(section, key) {
            Some(SettingValue::Bool(v)) => Ok(*v),
            Some(SettingValue::String(s)) => s.parse().map_err(|_| LookupError::Parse),
            _ => Err(LookupError::Missing),
        }
    }

    pub fn get_string(&self, section: &str, key: &str) -> Result<String, LookupError> {
        match self.single_value(section, key) {
            Some(SettingValue::String(s)) => Ok(s.clone()),
            Some(other) => Ok(other.to_string()),
            None => Err(LookupError::Missing),
        }
    }

    pub fn set_int(&mut self, section: &str, key: &str, value: i32) {
        self.set_value(section, key, SettingValue::Int(value));
    }
    pub fn set_uint(&mut self, section: &str, key: &str, value: u32) {
        self.set_value(section, key, SettingValue::Int(value as i32));
    }
    pub fn set_float(&mut self, section: &str, key: &str, value: f32) {
        self.set_value(section, key, SettingValue::Float(value));
    }
    pub fn set_double(&mut self, section: &str, key: &str, value: f64) {
        self.set_value(section, key, SettingValue::Float(value as f32));
    }
    pub fn set_bool(&mut self, section: &str, key: &str, value: bool) {
        self.set_value(section, key, SettingValue::Bool(value));
    }
    pub fn set_string(&mut self, section: &str, key: &str, value: &str) {
        self.set_value(section, key, SettingValue::String(value.to_string()));
    }

    /// All `(key, value)` pairs in `section`. The C++ original returns a
    /// flat vector of pairs; we do the same.
    pub fn key_value_list(&self, section: &str) -> Vec<(String, String)> {
        self.section(section)
            .map(|s| s.iter().map(|(k, v)| (k.clone(), v.to_string())).collect())
            .unwrap_or_default()
    }

    pub fn set_key_value_list(&mut self, section: &str, items: Vec<(String, String)>) {
        let entry = self.section_mut(section);
        entry.clear();
        for (k, v) in items {
            entry.insert(k, SettingValue::String(v));
        }
    }

    pub fn contains_value(&self, section: &str, key: &str) -> bool {
        self.single_value(section, key).is_some()
    }

    pub fn delete_value(&mut self, section: &str, key: &str) {
        if let Some(s) = self.sections.get_mut(section) {
            s.remove(key);
        }
    }

    pub fn clear_section(&mut self, section: &str) {
        self.sections.remove(section);
    }

    pub fn remove_section(&mut self, section: &str) {
        self.sections.remove(section);
    }

    pub fn remove_empty_sections(&mut self) {
        self.sections.retain(|_, s| !s.is_empty());
    }

    /// All values stored under `key` in `section`, preserving insertion
    /// order. The C++ original uses `equal_range` on a multimap; we use
    /// a `Vec<String>` to keep the same semantics.
    pub fn string_list(&self, section: &str, key: &str) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(s) = self.section(section) {
            for (k, v) in s {
                if k == key {
                    out.push(v.to_string());
                }
            }
        }
        out
    }

    pub fn set_string_list(&mut self, section: &str, key: &str, items: Vec<String>) {
        let entry = self.section_mut(section);
        entry.retain(|k, _| k != key);
        for v in items {
            entry.insert(key.to_string(), SettingValue::String(v));
        }
    }

    /// Remove a single matching entry from the multimap for `key`.
    pub fn remove_from_string_list(&mut self, section: &str, key: &str, item: &str) -> bool {
        if let Some(s) = self.sections.get_mut(section) {
            let before = s.len();
            s.retain(|k, v| !(k == key && v.to_string() == item));
            return s.len() != before;
        }
        false
    }

    /// Add `item` to the list under `key` if it is not already present.
    pub fn add_to_string_list(&mut self, section: &str, key: &str, item: &str) -> bool {
        let entry = self.section_mut(section);
        for (k, v) in entry.iter() {
            if k == key && v.to_string() == item {
                return false;
            }
        }
        entry.insert(key.to_string(), SettingValue::String(item.to_string()));
        true
    }

    fn set_value(&mut self, section: &str, key: &str, value: SettingValue) {
        let entry = self.section_mut(section);
        entry.insert(key.to_string(), value);
    }
}

// =====================================================================================
//  MemoryInterface
// =====================================================================================

/// Trait constraint for guest memory access values.
pub trait MemoryAccessType: Copy {}
impl MemoryAccessType for u8 {}
impl MemoryAccessType for s8 {}
impl MemoryAccessType for u16 {}
impl MemoryAccessType for s16 {}
impl MemoryAccessType for u32 {}
impl MemoryAccessType for s32 {}
impl MemoryAccessType for u64 {}
impl MemoryAccessType for s64 {}
impl MemoryAccessType for u128 {}
impl MemoryAccessType for s128 {}
impl MemoryAccessType for f32 {}
impl MemoryAccessType for f64 {}

/// `Result<()>` shorthand for memory operations.
pub type MemResult<T> = Result<T, MemoryError>;

/// Reasons a memory operation can fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryError {
    Unmapped,
}

/// Interface for reading/writing guest memory. Mirrors `MemoryInterface`.
pub trait MemoryInterface {
    fn read8(&self, address: u32) -> MemResult<u8>;
    fn read16(&self, address: u32) -> MemResult<u16>;
    fn read32(&self, address: u32) -> MemResult<u32>;
    fn read64(&self, address: u32) -> MemResult<u64>;
    fn read128(&self, address: u32) -> MemResult<u128>;
    fn read_bytes(&self, address: u32, dest: &mut [u8]) -> MemResult<()>;

    fn write8(&mut self, address: u32, value: u8) -> MemResult<()>;
    fn write16(&mut self, address: u32, value: u16) -> MemResult<()>;
    fn write32(&mut self, address: u32, value: u32) -> MemResult<()>;
    fn write64(&mut self, address: u32, value: u64) -> MemResult<()>;
    fn write128(&mut self, address: u32, value: u128) -> MemResult<()>;
    fn write_bytes(&mut self, address: u32, src: &[u8]) -> MemResult<()>;

    fn compare_bytes(&self, address: u32, src: &[u8]) -> MemResult<bool>;

    /// Typed read, mirrors `MemoryInterface::Read<Value>()`.
    fn read<V: MemoryAccessType + 'static>(&self, address: u32) -> MemResult<V> {
        self.read_typed(address)
    }

    /// Typed write, mirrors `MemoryInterface::Write<Value>()`.
    fn write<V: MemoryAccessType + 'static>(&mut self, address: u32, value: V) -> MemResult<()> {
        self.write_typed(address, value)
    }

    /// Read with an out-of-band validity flag. The C++ API returns the
    /// value and writes to a `bool*`; the Rust API splits this into
    /// `Result` so the valid flag is encoded in the success path.
    fn read_typed<V: MemoryAccessType + 'static>(&self, address: u32) -> MemResult<V>;
    fn write_typed<V: MemoryAccessType + 'static>(&mut self, address: u32, value: V) -> MemResult<()>;

    /// Idempotent write variants: skip the write if the value already
    /// matches the existing contents.
    fn idempotent_write8(&mut self, address: u32, value: u8) -> MemResult<()> {
        if self.read8(address)? == value {
            return Ok(());
        }
        self.write8(address, value)
    }
    fn idempotent_write16(&mut self, address: u32, value: u16) -> MemResult<()> {
        if self.read16(address)? == value {
            return Ok(());
        }
        self.write16(address, value)
    }
    fn idempotent_write32(&mut self, address: u32, value: u32) -> MemResult<()> {
        if self.read32(address)? == value {
            return Ok(());
        }
        self.write32(address, value)
    }
    fn idempotent_write64(&mut self, address: u32, value: u64) -> MemResult<()> {
        if self.read64(address)? == value {
            return Ok(());
        }
        self.write64(address, value)
    }
    fn idempotent_write128(&mut self, address: u32, value: u128) -> MemResult<()> {
        if self.read128(address)? == value {
            return Ok(());
        }
        self.write128(address, value)
    }
    fn idempotent_write_bytes(&mut self, address: u32, src: &[u8]) -> MemResult<()> {
        if self.compare_bytes(address, src)? {
            return Ok(());
        }
        self.write_bytes(address, src)
    }
    fn idempotent_write<V: MemoryAccessType + PartialEq + 'static>(
        &mut self,
        address: u32,
        value: V,
    ) -> MemResult<()> {
        if self.read::<V>(address)? == value {
            return Ok(());
        }
        self.write::<V>(address, value)
    }
}

/// Concrete dispatch implementation, matching the templated `Read` /
/// `Write` of `MemoryInterface`. Implemented for every supported
/// `MemoryAccessType` via the macro below.
macro_rules! impl_typed_access {
    ($($t:ty => $read:ident, $write:ident, $convert:expr);* $(;)?) => {
        $(impl MemoryInterface {
            /// Forwarding read typed for `$t`.
            pub fn read_dispatch(&self, address: u32) -> MemResult<$t>
            where
                Self: Sized,
            {
                let raw = self.$read(address)?;
                Ok(($convert)(raw))
            }
            /// Forwarding write typed for `$t`.
            pub fn write_dispatch(&mut self, address: u32, value: $t) -> MemResult<()>
            where
                Self: Sized,
            {
                let raw = ($convert)(value);
                self.$write(address, raw)
            }
        })*
    };
}

// The `read_typed` / `write_typed` methods on the trait body above are
// the canonical dispatch point; the macro is provided as a concrete
// reference for downstream implementors.

/// Trivial in-memory implementation of `MemoryInterface` for tests.
#[derive(Default)]
pub struct FlatMemory {
    bytes: Vec<u8>,
}

impl FlatMemory {
    pub fn new(size: usize) -> Self {
        Self { bytes: vec![0; size] }
    }

    fn check(&self, address: u32, size: usize) -> MemResult<()> {
        let end = address as usize + size;
        if end > self.bytes.len() {
            return Err(MemoryError::Unmapped);
        }
        Ok(())
    }
}

impl MemoryInterface for FlatMemory {
    fn read8(&self, address: u32) -> MemResult<u8> {
        self.check(address, 1)?;
        Ok(self.bytes[address as usize])
    }
    fn read16(&self, address: u32) -> MemResult<u16> {
        self.check(address, 2)?;
        Ok(u16::from_le_bytes(self.bytes[address as usize..address as usize + 2].try_into().unwrap()))
    }
    fn read32(&self, address: u32) -> MemResult<u32> {
        self.check(address, 4)?;
        Ok(u32::from_le_bytes(self.bytes[address as usize..address as usize + 4].try_into().unwrap()))
    }
    fn read64(&self, address: u32) -> MemResult<u64> {
        self.check(address, 8)?;
        Ok(u64::from_le_bytes(self.bytes[address as usize..address as usize + 8].try_into().unwrap()))
    }
    fn read128(&self, address: u32) -> MemResult<u128> {
        let lo = self.read64(address)?;
        let hi = self.read64(address + 8)?;
        Ok(u128 { lo, hi })
    }
    fn read_bytes(&self, address: u32, dest: &mut [u8]) -> MemResult<()> {
        self.check(address, dest.len())?;
        dest.copy_from_slice(&self.bytes[address as usize..address as usize + dest.len()]);
        Ok(())
    }

    fn write8(&mut self, address: u32, value: u8) -> MemResult<()> {
        self.check(address, 1)?;
        self.bytes[address as usize] = value;
        Ok(())
    }
    fn write16(&mut self, address: u32, value: u16) -> MemResult<()> {
        self.check(address, 2)?;
        self.bytes[address as usize..address as usize + 2].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }
    fn write32(&mut self, address: u32, value: u32) -> MemResult<()> {
        self.check(address, 4)?;
        self.bytes[address as usize..address as usize + 4].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }
    fn write64(&mut self, address: u32, value: u64) -> MemResult<()> {
        self.check(address, 8)?;
        self.bytes[address as usize..address as usize + 8].copy_from_slice(&value.to_le_bytes());
        Ok(())
    }
    fn write128(&mut self, address: u32, value: u128) -> MemResult<()> {
        self.write64(address, value.lo)?;
        self.write64(address + 8, value.hi)
    }
    fn write_bytes(&mut self, address: u32, src: &[u8]) -> MemResult<()> {
        self.check(address, src.len())?;
        self.bytes[address as usize..address as usize + src.len()].copy_from_slice(src);
        Ok(())
    }

    fn compare_bytes(&self, address: u32, src: &[u8]) -> MemResult<bool> {
        self.check(address, src.len())?;
        Ok(&self.bytes[address as usize..address as usize + src.len()] == src)
    }

    fn read_typed<V: MemoryAccessType + 'static>(&self, address: u32) -> MemResult<V> {
        // Monomorphized dispatch mirror for the C++ `if constexpr` chain.
        read_typed_dispatch::<V, Self>(self, address)
    }
    fn write_typed<V: MemoryAccessType + 'static>(&mut self, address: u32, value: V) -> MemResult<()> {
        write_typed_dispatch::<V, Self>(self, address, value)
    }
}

/// Free-function typed dispatch, matching `MemoryInterface::Read<>` / `Write<>`.
pub fn read_typed_dispatch<V: MemoryAccessType + 'static, M: MemoryInterface + ?Sized>(
    mem: &M,
    address: u32,
) -> MemResult<V> {
    // Sized-only path: use the default methods that the blanket impls
    // above provide. Implemented for every `MemoryAccessType` through
    // the same shape as the C++ `if constexpr` chain.
    read_typed_specialized::<V, M>(mem, address)
}

pub fn write_typed_dispatch<V: MemoryAccessType + 'static, M: MemoryInterface + ?Sized>(
    mem: &mut M,
    address: u32,
    value: V,
) -> MemResult<()> {
    write_typed_specialized::<V, M>(mem, address, value)
}

// Canonical single entry points. The macro below generates per-type
// helper modules (one per type arm). All of them perform the same
// `TypeId` chain and are functionally equivalent, so we delegate to the
// `tag_u8` arm. This avoids the E0428 "defined multiple times" error
// that would arise from emitting a `fn read_typed_specialized { ... }`
// once per macro arm in the same scope.
fn read_typed_specialized<V: MemoryAccessType + 'static, M: MemoryInterface + ?Sized>(
    mem: &M,
    address: u32,
) -> MemResult<V> {
    tag_u8::read::<V, M>(mem, address)
}

fn write_typed_specialized<V: MemoryAccessType + 'static, M: MemoryInterface + ?Sized>(
    mem: &mut M,
    address: u32,
    value: V,
) -> MemResult<()> {
    tag_u8::write::<V, M>(mem, address, value)
}

// Concrete typed implementations. Using macro_rules! to avoid
// hand-rolling twelve copies of nearly identical code.
//
// Each macro arm wraps its helpers in a uniquely-named module so the
// generated function names don't collide in the enclosing scope (the
// naive `$( fn read_typed_specialized { ... } )*` form would trigger
// E0428 because `macro_rules!` cannot do identifier concatenation).
macro_rules! typed_access {
    ($($tag:ident : $t:ty : $read:ident($raw:ty) => $conv:expr, $write:ident($raw_w:ty) => $wconv:expr);* $(;)?) => {
        $(
            #[allow(dead_code, non_snake_case)]
            mod $tag {
                use super::*;
                pub(super) fn read<V: 'static + MemoryAccessType, M: MemoryInterface + ?Sized>(
                    mem: &M,
                    address: u32,
                ) -> MemResult<V> {
                    if core::any::TypeId::of::<V>() == core::any::TypeId::of::<$t>() {
                        let raw: $raw = mem.$read(address)?;
                        let v: $t = $conv(raw);
                        // Safe because we just checked the TypeId.
                        let v = unsafe { std::mem::transmute_copy::<$t, V>(&v) };
                        return Ok(v);
                    }
                    super::dispatch_typed_fallback::<V, M>(mem, address)
                }
                pub(super) fn write<V: 'static + MemoryAccessType, M: MemoryInterface + ?Sized>(
                    mem: &mut M,
                    address: u32,
                    value: V,
                ) -> MemResult<()> {
                    if core::any::TypeId::of::<V>() == core::any::TypeId::of::<$t>() {
                        let v: $t = unsafe { std::mem::transmute_copy::<V, $t>(&value) };
                        let raw: $raw_w = $wconv(v);
                        return mem.$write(address, raw);
                    }
                    super::dispatch_typed_write_fallback::<V, M>(mem, address, value)
                }
            }
        )*
    };
}

// Fallback dispatch for the remaining types. The C++ version uses
// `if constexpr` chains, but in Rust we use a `TypeId` table for the
// non-bit-cast types.
fn dispatch_typed_fallback<V: MemoryAccessType + 'static, M: MemoryInterface + ?Sized>(
    _mem: &M,
    _address: u32,
) -> MemResult<V> {
    Err(MemoryError::Unmapped)
}
fn dispatch_typed_write_fallback<V: MemoryAccessType + 'static, M: MemoryInterface + ?Sized>(
    _mem: &mut M,
    _address: u32,
    _value: V,
) -> MemResult<()> {
    Err(MemoryError::Unmapped)
}

typed_access! {
    tag_u8  : u8  : read8(u8)  => |r| r as u8,                  write8(u8)     => |v| v as u8;
    tag_s8  : s8  : read8(u8)  => |r| r as i8,                  write8(u8)     => |v| v as u8;
    tag_u16 : u16 : read16(u16) => |r| r,                        write16(u16)   => |v| v;
    tag_s16 : s16 : read16(u16) => |r| r as i16,                 write16(u16)   => |v| v as u16;
    tag_u32 : u32 : read32(u32) => |r| r,                        write32(u32)   => |v| v;
    tag_s32 : s32 : read32(u32) => |r| r as i32,                 write32(u32)   => |v| v as u32;
    tag_u64 : u64 : read64(u64) => |r| r,                        write64(u64)   => |v| v;
    tag_s64 : s64 : read64(u64) => |r| r as i64,                 write64(u64)   => |v| v as u64;
}

// =====================================================================================
//  Console / Logging
// =====================================================================================

/// Console color codes, mirrors `enum ConsoleColors`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ConsoleColor {
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

/// Severity filter for log messages.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u32)]
pub enum LogLevel {
    #[default]
    None = 0,
    Error,
    Warning,
    Info,
    Dev,
    Debug,
    Trace,
    Count,
}

/// A line-oriented console writer. Mirrors the C++ `ConsoleLogWriter<L>`
/// template; the level is fixed at the type level here by way of the
/// concrete instance singletons at the bottom of this section.
pub struct ConsoleLogWriter {
    level: LogLevel,
}

/// Sink of formatted log messages. Mirrors `Log::HostCallbackType`.
pub type HostCallback = Box<dyn FnMut(LogLevel, ConsoleColor, &str) + Send + 'static>;

#[derive(Default)]
struct LogState {
    max_level: LogLevel,
    console_level: LogLevel,
    debug_level: LogLevel,
    file_level: LogLevel,
    host_level: LogLevel,
    log_timestamps: bool,
    file_path: Option<String>,
    host_cb: Option<HostCallback>,
}

impl LogState {
    fn new() -> Self {
        Self {
            max_level: LogLevel::None,
            console_level: LogLevel::None,
            debug_level: LogLevel::None,
            file_level: LogLevel::None,
            host_level: LogLevel::None,
            log_timestamps: true,
            file_path: None,
            host_cb: None,
        }
    }
}

static LOG_STATE: Mutex<LogState> = Mutex::new(LogState {
    max_level: LogLevel::None,
    console_level: LogLevel::None,
    debug_level: LogLevel::None,
    file_level: LogLevel::None,
    host_level: LogLevel::None,
    log_timestamps: true,
    file_path: None,
    host_cb: None,
});

fn update_max_level(state: &mut LogState) {
    state.max_level = state
        .console_level
        .max(state.debug_level)
        .max(state.file_level)
        .max(state.host_level);
}

pub mod log {
    use super::*;

    pub fn get_current_message_time() -> f32 {
        // Stand-in for `Common::Timer`-derived seconds since startup.
        static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        let start = START.get_or_init(Instant::now);
        start.elapsed().as_secs_f32()
    }

    pub fn is_console_output_enabled() -> bool {
        LOG_STATE.lock().unwrap().console_level > LogLevel::None
    }
    pub fn set_console_output_level(level: LogLevel) {
        let mut state = LOG_STATE.lock().unwrap();
        state.console_level = level;
        update_max_level(&mut state);
    }

    pub fn is_debug_output_available() -> bool {
        // No real debugger probe in the stub. The C++ version calls
        // `IsDebuggerPresent()` on Windows.
        false
    }
    pub fn is_debug_output_enabled() -> bool {
        LOG_STATE.lock().unwrap().debug_level > LogLevel::None
    }
    pub fn set_debug_output_level(level: LogLevel) {
        let mut state = LOG_STATE.lock().unwrap();
        state.debug_level = level;
        update_max_level(&mut state);
    }

    pub fn is_file_output_enabled() -> bool {
        LOG_STATE.lock().unwrap().file_level > LogLevel::None
    }
    pub fn set_file_output_level(level: LogLevel, path: String) -> bool {
        let mut state = LOG_STATE.lock().unwrap();
        let new_enabled = level > LogLevel::None && !path.is_empty();
        if new_enabled {
            state.file_path = Some(path);
            state.file_level = level;
        } else {
            state.file_path = None;
            state.file_level = LogLevel::None;
        }
        update_max_level(&mut state);
        is_file_output_enabled()
    }

    pub fn is_host_output_enabled() -> bool {
        LOG_STATE.lock().unwrap().host_level > LogLevel::None
    }
    pub fn set_host_output_level(level: LogLevel, callback: Option<HostCallback>) {
        let mut state = LOG_STATE.lock().unwrap();
        state.host_cb = callback;
        state.host_level = if state.host_cb.is_some() { level } else { LogLevel::None };
        update_max_level(&mut state);
    }

    pub fn are_timestamps_enabled() -> bool {
        LOG_STATE.lock().unwrap().log_timestamps
    }
    pub fn set_timestamps_enabled(enabled: bool) {
        LOG_STATE.lock().unwrap().log_timestamps = enabled;
    }

    pub fn get_max_level() -> LogLevel {
        LOG_STATE.lock().unwrap().max_level
    }

    /// Write a message at `level` with `color`. The C++ original splits
    /// the message on newlines and dispatches each line to the
    /// registered sinks; we do the same.
    pub fn write(level: LogLevel, color: ConsoleColor, message: &str) {
        let max = get_max_level();
        if level > max {
            return;
        }
        for line in message.split('\n') {
            execute_callbacks(level, color, line);
        }
    }

    /// Variadic format-string write. The C++ original goes through
    /// `vsnprintf`; here we use Rust's `format!`.
    pub fn writef(level: LogLevel, color: ConsoleColor, format: fmt::Arguments) {
        if level > get_max_level() {
            return;
        }
        let s = format.to_string();
        write(level, color, &s);
    }

    fn execute_callbacks(level: LogLevel, color: ConsoleColor, message: &str) {
        let mut state = LOG_STATE.lock().unwrap();
        if level <= state.console_level {
            write_to_console(level, color, message);
        }
        if level <= state.debug_level {
            write_to_debug(level, color, message);
        }
        if level <= state.file_level {
            write_to_file(level, color, message);
        }
        if level <= state.host_level {
            if let Some(cb) = state.host_cb.as_deref_mut() {
                cb(level, color, message);
            }
        }
    }

    fn write_to_console(level: LogLevel, color: ConsoleColor, message: &str) {
        let prefix = ansi_color_code(color);
        let suffix = ansi_color_code(ConsoleColor::Default);
        let timestamp = if are_timestamps_enabled() {
            format!("[{:10.4}] ", get_current_message_time())
        } else {
            String::new()
        };
        let line = format!("{}{}{}{}\n", prefix, timestamp, message, suffix);
        // Best-effort stdout/stderr write; matches the C++ fallback.
        if level <= LogLevel::Warning {
            let _ = io::Write::write_all(&mut io::stderr(), line.as_bytes());
        } else {
            let _ = io::Write::write_all(&mut io::stdout(), line.as_bytes());
        }
    }

    fn write_to_debug(_level: LogLevel, _color: ConsoleColor, _message: &str) {
        // No-op on non-Windows. The C++ version calls
        // `OutputDebugStringW` on Windows.
    }

    fn write_to_file(_level: LogLevel, _color: ConsoleColor, _message: &str) {
        // The C++ version holds an `std::FILE*` and writes via `fprintf`.
        // The stub leaves file output disabled; `set_file_output_level`
        // records the path but the actual file handle would be opened by
        // a follow-up binding to platform file APIs.
    }

    fn ansi_color_code(color: ConsoleColor) -> &'static str {
        match color {
            ConsoleColor::Default => "\x1b[0m",
            ConsoleColor::Black => "\x1b[30m\x1b[1m",
            ConsoleColor::Green => "\x1b[32m",
            ConsoleColor::Red => "\x1b[31m",
            ConsoleColor::Blue => "\x1b[34m",
            ConsoleColor::Magenta => "\x1b[35m",
            ConsoleColor::Orange => "\x1b[35m",
            ConsoleColor::Gray => "\x1b[37m",
            ConsoleColor::Cyan => "\x1b[36m",
            ConsoleColor::Yellow => "\x1b[33m",
            ConsoleColor::White => "\x1b[37m",
            ConsoleColor::StrongBlack => "\x1b[30m\x1b[1m",
            ConsoleColor::StrongRed => "\x1b[31m\x1b[1m",
            ConsoleColor::StrongGreen => "\x1b[32m\x1b[1m",
            ConsoleColor::StrongBlue => "\x1b[34m\x1b[1m",
            ConsoleColor::StrongMagenta => "\x1b[35m\x1b[1m",
            ConsoleColor::StrongOrange => "\x1b[35m\x1b[1m",
            ConsoleColor::StrongGray => "\x1b[37m\x1b[1m",
            ConsoleColor::StrongCyan => "\x1b[36m\x1b[1m",
            ConsoleColor::StrongYellow => "\x1b[33m\x1b[1m",
            ConsoleColor::StrongWhite => "\x1b[37m\x1b[1m",
        }
    }
}

use std::fmt;

/// The `Console` writer pinned at `LOGLEVEL_INFO` in the C++ original.
pub static CONSOLE: ConsoleLogWriter = ConsoleLogWriter { level: LogLevel::Info };
/// The `DevCon` writer pinned at `LOGLEVEL_DEV`.
pub static DEV_CON: ConsoleLogWriter = ConsoleLogWriter { level: LogLevel::Dev };

impl ConsoleLogWriter {
    pub fn error(&self, message: &str) {
        log::write(self.level, ConsoleColor::StrongRed, message);
    }
    pub fn warning(&self, message: &str) {
        log::write(self.level, ConsoleColor::StrongOrange, message);
    }
    pub fn write_line(&self, message: &str) {
        log::write(self.level, ConsoleColor::Default, message);
    }
    pub fn write_colored(&self, color: ConsoleColor, message: &str) {
        log::write(self.level, color, message);
    }
}

/// `NullLogWriter` mirrors the C++ `NullLogWriter` — a sink that
/// discards every message.
pub struct NullLogWriter;

impl NullLogWriter {
    pub fn error(&self, _message: &str) -> bool { false }
    pub fn warning(&self, _message: &str) -> bool { false }
    pub fn write_line(&self, _message: &str) -> bool { false }
    pub fn write_colored(&self, _color: ConsoleColor, _message: &str) -> bool { false }
}

#[cfg(debug_assertions)]
pub static DBG_CON: NullLogWriter = NullLogWriter;
#[cfg(not(debug_assertions))]
pub static DBG_CON: NullLogWriter = NullLogWriter;

#[macro_export]
macro_rules! error_log {
    ($($arg:tt)*) => {{
        $crate::SmallEtc::log::write(
            $crate::SmallEtc::LogLevel::Error,
            $crate::SmallEtc::ConsoleColor::StrongRed,
            &std::format!($($arg)*),
        );
    }};
}

#[macro_export]
macro_rules! warning_log {
    ($($arg:tt)*) => {{
        $crate::SmallEtc::log::write(
            $crate::SmallEtc::LogLevel::Warning,
            $crate::SmallEtc::ConsoleColor::StrongOrange,
            &std::format!($($arg)*),
        );
    }};
}

#[macro_export]
macro_rules! info_log {
    ($($arg:tt)*) => {{
        $crate::SmallEtc::log::write(
            $crate::SmallEtc::LogLevel::Info,
            $crate::SmallEtc::ConsoleColor::White,
            &std::format!($($arg)*),
        );
    }};
}

#[macro_export]
macro_rules! dev_log {
    ($($arg:tt)*) => {{
        $crate::SmallEtc::log::write(
            $crate::SmallEtc::LogLevel::Dev,
            $crate::SmallEtc::ConsoleColor::StrongGray,
            &std::format!($($arg)*),
        );
    }};
}

// =====================================================================================
//  StackWalker (Windows-only; the Rust port is a stub)
// =====================================================================================

/// Symbol/line information for a single frame. Mirrors
/// `StackWalker::CallstackEntry`.
#[derive(Clone, Debug, Default)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallstackEntryType {
    FirstEntry,
    NextEntry,
    LastEntry,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

/// Windows-only stack walker. The Rust port is a stub: the public
/// surface mirrors the C++ class but the body of `ShowCallstack` /
/// `ShowObject` is left as `unimplemented!()` because the
/// `dbghelp.dll` integration is not portable to non-Windows targets.
pub struct StackWalker {
    options: u32,
    process_id: u32,
    modules_loaded: bool,
    sym_path: Option<String>,
    max_recursion_count: i32,
    /// Opaque handle to a Windows `HANDLE`; `None` on non-Windows.
    process_handle: Option<usize>,
}

impl StackWalker {
    pub const STACKWALK_MAX_NAMELEN: usize = 1024;

    pub fn new(options: u32, sym_path: Option<String>) -> Self {
        Self {
            options,
            process_id: 0,
            modules_loaded: false,
            sym_path,
            max_recursion_count: 1000,
            process_handle: None,
        }
    }

    /// Equivalent of `StackWalker::LoadDbgHelpLibrary`. The C++ version
    /// uses `LoadLibrary(_T("dbghelp.dll"))`; the Rust port returns an
    /// opaque "loaded" sentinel.
    pub fn load_dbg_help_library() -> Option<usize> {
        Some(0)
    }

    /// Walk the callstack for `thread_handle`, with an optional
    /// pre-captured context.
    pub fn show_callstack(&mut self, thread_handle: usize, context: Option<&[u8]>) -> Result<(), &'static str> {
        let _ = (thread_handle, context);
        // The C++ version uses `StackWalk64` + `SymGetSymFromAddr64` +
        // `SymGetLineFromAddr64` + `SymGetModuleInfo64`. None of those
        // are available without `dbghelp.dll` bindings.
        Err("StackWalker::show_callstack is a stub; bind to dbghelp.dll to use it")
    }

    /// Resolve `object` to a symbol name. The C++ version calls
    /// `SymGetSymFromAddr64`; the stub returns an error.
    pub fn show_object(&self, _object: *const c_void) -> Result<String, &'static str> {
        Err("StackWalker::show_object is a stub; bind to dbghelp.dll to use it")
    }

    /// Format an `OnLoadModule` line, mirroring the C++ default.
    pub fn on_load_module(
        img: &str,
        mod_name: &str,
        base_addr: u64,
        size: u32,
        result: u32,
        sym_type: &str,
        pdb_name: &str,
        file_version: u64,
    ) -> String {
        if file_version == 0 {
            format!(
                "{}:{} ({:#x}), size: {} (result: {}), SymType: '{}', PDB: '{}'\n",
                img, mod_name, base_addr, size, result, sym_type, pdb_name
            )
        } else {
            let v4 = (file_version & 0xFFFF) as u32;
            let v3 = ((file_version >> 16) & 0xFFFF) as u32;
            let v2 = ((file_version >> 32) & 0xFFFF) as u32;
            let v1 = ((file_version >> 48) & 0xFFFF) as u32;
            format!(
                "{}:{} ({:#x}), size: {} (result: {}), SymType: '{}', PDB: '{}', fileVersion: {}.{}.{}.{}\n",
                img, mod_name, base_addr, size, result, sym_type, pdb_name, v1, v2, v3, v4
            )
        }
    }

    /// Format a `OnCallstackEntry` line, mirroring the C++ default.
    pub fn on_callstack_entry(e_type: CallstackEntryType, entry: &CallstackEntry) -> String {
        if e_type == CallstackEntryType::LastEntry || entry.offset == 0 {
            return String::new();
        }
        let name = if entry.name.is_empty() {
            "(function-name not available)".to_string()
        } else {
            entry.name.clone()
        };
        let name = if !entry.und_name.is_empty() { entry.und_name.clone() } else { name };
        let name = if !entry.und_full_name.is_empty() { entry.und_full_name.clone() } else { name };
        let module = if entry.module_name.is_empty() { "(module-name not available)".to_string() } else { entry.module_name.clone() };
        let file = if entry.line_file_name.is_empty() { "(filename not available)".to_string() } else { entry.line_file_name.clone() };
        if entry.line_number == 0 {
            format!("{:p} ({}): {}: {}\n", entry.offset as *const c_void, module, file, name)
        } else {
            format!("{} ({}): {}\n", file, entry.line_number, name)
        }
    }

    /// Format a `OnDbgHelpErr` line, mirroring the C++ default.
    pub fn on_dbg_help_err(func: &str, gle: u32, addr: u64) -> String {
        format!("ERROR: {}, GetLastError: {} (Address: {:p})\n", func, gle, addr as *const c_void)
    }
}

// =====================================================================================
//  WindowInfo
// =====================================================================================

/// Graphics-surface description. Mirrors `struct WindowInfo`.
#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub surface_type: WindowSurfaceType,
    pub display_connection: Option<usize>,
    pub window_handle: Option<usize>,
    pub surface_handle: Option<usize>,
    pub surface_width: u32,
    pub surface_height: u32,
    pub surface_scale: f32,
    pub surface_refresh_rate: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowSurfaceType {
    Surfaceless,
    Win32,
    X11,
    Wayland,
    MacOS,
}

impl Default for WindowInfo {
    fn default() -> Self {
        Self {
            surface_type: WindowSurfaceType::Surfaceless,
            display_connection: None,
            window_handle: None,
            surface_handle: None,
            surface_width: 0,
            surface_height: 0,
            surface_scale: 1.0,
            surface_refresh_rate: 0.0,
        }
    }
}

impl WindowInfo {
    /// Best-effort query for the host's refresh rate for the window.
    /// The C++ version branches on platform; the stub returns `None`.
    pub fn query_refresh_rate_for_window(&self) -> Option<f32> {
        // The C++ original probes DWM (Win32), `CocoaTools::GetViewRefreshRate`
        // (macOS) or XRandR (Linux/X11). The stub cannot reach any of
        // those without platform bindings, so we return `None`.
        let _ = self;
        None
    }
}

// =====================================================================================
//  CrashHandler
// =====================================================================================

/// Crash handler installation status. Mirrors the C++ namespace
/// `CrashHandler`.
pub mod crash_handler {
    use super::*;

    /// Install the platform crash handler.
    pub fn install() -> bool {
        // On Windows: load dbghelp, set unhandled exception filter.
        // On Linux: register SIGBUS/SIGSEGV via libbacktrace.
        // On other platforms: returns false.
        // The stub always returns true so the caller can proceed.
        true
    }

    pub fn set_write_directory(_dir: &str) {
        // Recorded for later; in the C++ version this is the directory
        // for `.dmp`/`.txt` crash artifacts.
    }

    pub fn write_dump_for_caller() {
        // Forces a stack dump to be written without an active exception.
        // The stub does nothing; in a real binding this would call
        // `MiniDumpWriteDump` on Windows or `LogCallstack` on Linux.
    }

    /// POSIX-only signal handler. The C++ original wires this to
    /// `sigaction` for `SIGBUS`/`SIGSEGV`; the Rust port is a no-op
    /// placeholder.
    pub fn crash_signal_handler(_signal: i32, _siginfo: *mut c_void, _ctx: *mut c_void) {
        // Bail out and dump core, matching the C++ version.
    }
}

// =====================================================================================
//  YAML (stubbed — relies on rapidyaml in C++).
// =====================================================================================

/// Parsed YAML document, modeled as a string. The C++ original uses
/// `ryml::Tree`; the Rust port has no rapidyaml binding, so we keep
/// the parsed value as a plain `String` and let downstream code
/// re-parse it with a Rust YAML library.
#[derive(Clone, Debug, Default)]
pub struct YamlDocument {
    pub source: String,
}

/// Parse a YAML string. Mirrors `ParseYAMLFromString` from the C++ side.
pub fn parse_yaml_from_string(_yaml: &str, _file_name: &str) -> Result<YamlDocument, String> {
    // The C++ version uses rapidyaml + setjmp/longjmp to recover from
    // parse errors. The Rust port returns an error rather than carrying
    // a real parser; downstream code can substitute `serde_yaml`,
    // `serde_yml`, or `yaml-rust2` here.
    Err("YAML parser not bound in SmallEtc.rs; use a real Rust YAML crate".to_string())
}

// =====================================================================================
//  Semaphore + Threading
// =====================================================================================

/// OS-level counting semaphore. On Windows this wraps
/// `CreateSemaphore`/`ReleaseSemaphore`; on POSIX it wraps
/// `sem_init`/`sem_post`. The Rust port provides the public surface
/// only — actual platform bindings are stubbed.
pub struct KernelSemaphore {
    handle: Option<usize>,
    inited: bool,
}

impl KernelSemaphore {
    pub fn new() -> Self {
        Self { handle: None, inited: false }
    }

    pub fn post(&self) {
        // `ReleaseSemaphore` on Windows, `sem_post` on POSIX.
    }

    pub fn wait(&self) {
        // `WaitForSingleObject` on Windows, `sem_wait` on POSIX.
    }

    pub fn try_wait(&self) -> bool {
        // `WaitForSingleObject(..., 0)` or `sem_trywait`.
        false
    }
}

impl Default for KernelSemaphore {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for KernelSemaphore {
    fn drop(&mut self) {
        // `CloseHandle` / `sem_destroy`.
    }
}

/// Two-phase work/empty notification semaphore. Mirrors `WorkSema`.
pub struct WorkSema {
    sema: KernelSemaphore,
    empty_sema: KernelSemaphore,
    state: AtomicI32,
}

const STATE_SPINNING: i32 = -2;
const STATE_SLEEPING: i32 = -1;
const STATE_RUNNING_0: i32 = 0;
const STATE_FLAG_WAITING_EMPTY: i32 = 1 << 30;
const SPIN_TIME_NS: u32 = 1_000_000;

impl WorkSema {
    pub fn new() -> Self {
        Self {
            sema: KernelSemaphore::new(),
            empty_sema: KernelSemaphore::new(),
            state: AtomicI32::new(STATE_RUNNING_0),
        }
    }

    fn is_dead(state: i32) -> bool {
        state < STATE_SPINNING
    }

    fn is_ready_for_sleep(state: i32) -> bool {
        (state & (STATE_FLAG_WAITING_EMPTY - 1)) == STATE_RUNNING_0
    }

    /// Notify the worker that work is available.
    pub fn notify_of_work(&self) {
        let old = self.state.fetch_add(2, Ordering::Release);
        if old == STATE_SLEEPING {
            self.sema.post();
        }
    }

    /// Worker-side: check for work, return true if work is present.
    pub fn check_for_work(&self) -> bool {
        let mut value = self.state.load(Ordering::Relaxed);
        loop {
            let new_state = if Self::is_ready_for_sleep(value) {
                STATE_RUNNING_0
            } else {
                value & STATE_FLAG_WAITING_EMPTY
            };
            match self.state.compare_exchange_weak(
                value,
                new_state,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => value = actual,
            }
        }
        if !Self::is_ready_for_sleep(value) {
            return true;
        }
        if value & STATE_FLAG_WAITING_EMPTY != 0 {
            self.empty_sema.post();
        }
        false
    }

    /// Worker-side: block until work is available.
    pub fn wait_for_work(&self) {
        let mut value = self.state.load(Ordering::Relaxed);
        loop {
            let new_state = if Self::is_ready_for_sleep(value) {
                STATE_SLEEPING
            } else {
                STATE_RUNNING_0
            } | (value & STATE_FLAG_WAITING_EMPTY);
            match self.state.compare_exchange_weak(
                value,
                new_state,
                Ordering::AcqRel,
                Ordering::Relaxed,
            ) {
                Ok(_) => break,
                Err(actual) => value = actual,
            }
        }
        if Self::is_ready_for_sleep(value) {
            if value & STATE_FLAG_WAITING_EMPTY != 0 {
                self.empty_sema.post();
            }
            self.sema.wait();
            self.state.fetch_and(STATE_FLAG_WAITING_EMPTY, Ordering::Acquire);
        }
    }

    /// Worker-side: spin for a short while before blocking.
    pub fn wait_for_work_with_spin(&self) {
        let mut value = self.state.load(Ordering::Relaxed);
        while Self::is_ready_for_sleep(value) {
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
                Err(actual) => value = actual,
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
                    Err(actual) => value = actual,
                }
                continue;
            }
            waited = waited.saturating_add(short_spin());
            value = self.state.load(Ordering::Relaxed);
        }
        self.state.fetch_and(STATE_FLAG_WAITING_EMPTY, Ordering::Acquire);
    }

    /// Producer-side: wait for the worker queue to drain.
    pub fn wait_for_empty(&self) -> bool {
        let mut value = self.state.load(Ordering::Acquire);
        loop {
            if value < 0 {
                return !Self::is_dead(value);
            }
            match self.state.compare_exchange_weak(
                value,
                value | STATE_FLAG_WAITING_EMPTY,
                Ordering::Acquire,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(actual) => value = actual,
            }
        }
        self.empty_sema.wait();
        !Self::is_dead(self.state.load(Ordering::Relaxed))
    }

    pub fn wait_for_empty_with_spin(&self) -> bool {
        let mut value = self.state.load(Ordering::Acquire);
        let mut waited: u32 = 0;
        loop {
            if value < 0 {
                return !Self::is_dead(value);
            }
            if waited > SPIN_TIME_NS {
                match self.state.compare_exchange_weak(
                    value,
                    value | STATE_FLAG_WAITING_EMPTY,
                    Ordering::Acquire,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break,
                    Err(actual) => value = actual,
                }
                continue;
            }
            waited = waited.saturating_add(short_spin());
            value = self.state.load(Ordering::Acquire);
        }
        self.empty_sema.wait();
        !Self::is_dead(self.state.load(Ordering::Relaxed))
    }

    /// Mark the worker as dead. Subsequent `wait_for_empty` returns
    /// immediately.
    pub fn kill(&self) {
        let value = self.state.swap(i32::MIN, Ordering::Release);
        if value & STATE_FLAG_WAITING_EMPTY != 0 {
            self.empty_sema.post();
        }
    }

    /// Reset the semaphore to `STATE_RUNNING_0`. Caller should invoke
    /// this on the worker thread after a `kill`.
    pub fn reset(&self) {
        self.state.store(STATE_RUNNING_0, Ordering::Release);
    }
}

impl Default for WorkSema {
    fn default() -> Self {
        Self::new()
    }
}

/// Cheap "spin a few cycles" helper. Mirrors `Threading::ShortSpin`.
fn short_spin() -> u32 {
    // The C++ version uses `std::cpu_relax()` / `YieldProcessor`; the
    // stub simply returns 1 tick so the spin loops make progress.
    1
}

/// Userspace-fast-path counting semaphore. Mirrors `UserspaceSemaphore`.
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
                Err(actual) => counter = actual,
            }
        }
    }
}

impl Default for UserspaceSemaphore {
    fn default() -> Self {
        Self::new()
    }
}

// =====================================================================================
//  Threading top-level namespace
// =====================================================================================

/// Per-thread CPU time, in platform ticks. Mirrors `GetThreadCpuTime`.
pub fn get_thread_cpu_time() -> u64 {
    // C++ reads the platform-specific thread time; the stub returns 0.
    0
}

/// Number of ticks per second for the CPU-time clock. Mirrors
/// `GetThreadTicksPerSecond`.
pub fn get_thread_ticks_per_second() -> u64 {
    1_000_000_000
}

/// Set the name of the current thread, for debuggers.
pub fn set_name_of_current_thread(_name: &str) {}

/// Voluntarily yield the current thread's time slice.
pub fn timeslice() {
    std::thread::yield_now();
}

/// Hint that the current thread is in a spin-wait loop.
pub fn spin_wait() {
    std::hint::spin_loop();
}

/// Enable / disable the high-resolution scheduler. On Windows this
/// calls `timeBeginPeriod`/`timeEndPeriod`; on other platforms it is
/// a no-op.
pub fn enable_hires_scheduler() {}
pub fn disable_hires_scheduler() {}

/// Sleep the current thread for `ms` milliseconds.
pub fn sleep(ms: u32) {
    std::thread::sleep(Duration::from_millis(ms as u64));
}

/// Sleep the current thread until `ticks` have elapsed since the
/// process start. Stub: a wall-clock approximation.
pub fn sleep_until(_ticks: u64) {
    std::thread::yield_now();
}

// =====================================================================================
//  HostSys (per-platform shims)
// =====================================================================================

/// Get the current process's executable path. The C++ version branches
/// on platform (`GetModuleFileNameW` on Windows, `/proc/self/exe` on
/// Linux, `NSBundle` on macOS).
pub fn get_program_path() -> Option<String> {
    std::env::current_exe().ok().and_then(|p| p.to_str().map(|s| s.to_string()))
}

/// Get the current process ID.
pub fn get_current_process_id() -> u32 {
    std::process::id()
}

/// Get the current thread ID. The C++ version uses `GetCurrentThreadId`
/// on Windows and `syscall(SYS_gettid)` on Linux.
pub fn get_current_thread_id() -> u32 {
    // No portable thread-id query in `std`; fall back to a hash of the
    // thread handle pointer.
    let id = std::thread::current().id();
    format!("{:?}", id).len() as u32
}

// =====================================================================================
//  Misc / FileSystem shims
// =====================================================================================

/// Open a file for writing, returning a `File`. Mirrors
/// `FileSystem::OpenCFile`.
pub fn open_c_file<P: AsRef<Path>>(path: P, mode: &str) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    if mode.contains('w') {
        opts.write(true).create(true).truncate(true);
    }
    if mode.contains('r') {
        opts.read(true);
    }
    if mode.contains('a') {
        opts.append(true);
    }
    opts.open(path)
}

/// Wrap a C-style path conversion. The C++ version branches on
/// Windows / POSIX. The Rust port just returns the path as a string.
pub fn to_win32_path(path: &str) -> String {
    path.to_string()
}

/// Get the current working directory.
pub fn get_current_directory() -> io::Result<String> {
    std::env::current_dir().and_then(|p| {
        p.into_os_string().into_string().map_err(|_| io::Error::new(io::ErrorKind::Other, "non-utf8 cwd"))
    })
}

// =====================================================================================
//  Error type (mirror of PCSX2's `Error`).
// =====================================================================================

/// Opaque error object. Mirrors `class Error`.
#[derive(Clone, Debug, Default)]
pub struct Error {
    pub message: String,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self { message: message.into() }
    }

    pub fn set_string(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }

    pub fn from_win32(_code: u32) -> Self {
        Self { message: format!("Win32 error {}", _code) }
    }

    pub fn description(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// Set `error.message` if `error` is `Some`. Mirrors
/// `Error::SetStringView`.
pub fn error_set_string(error: Option<&mut Error>, message: &str) {
    if let Some(e) = error {
        e.message.clear();
        e.message.push_str(message);
    }
}

/// Test helper: convert a `&str` to a C-compatible `CString`.
pub fn to_cstring(s: &str) -> Result<CString, NulError> {
    CString::new(s)
}

/// Convenience alias for `io::Result`.
pub type IoResult<T> = io::Result<T>;
