//! `error` — Idiomatic Rust port of PCSX2's C++ `common/Error.{h,cpp}`.
//!
//! PCSX2 traditionally threads errors through the call stack by passing a
//! nullable `Error*` to every fallible function and inspecting it on the
//! other side:
//!
//! ```cpp
//! std::optional<Foo> DoThing(Error* err);
//! auto f = DoThing(&err);
//! if (!err.IsValid()) { /* success */ }
//! ```
//!
//! This module preserves that idiom for the C ABI while also exposing the
//! type in two idiomatic Rust shapes:
//!
//! 1. **`Pcsx2Error`** — a plain Rust enum carrying the kind and the
//!    native error code (or message). This is the natural form to use in
//!    `Result<T, Pcsx2Error>`. It also `impl std::error::Error`, so it
//!    slots into the wider Rust error ecosystem.
//!
//! 2. **`Error`** — a small wrapper around `Pcsx2Error` that mirrors the
//!    C++ class: copy/move semantics, an `is_valid()` flag, mutating
//!    `set_*` methods, prefix/suffix decoration, and the static helpers
//!    that take a `Option<&mut Error>` (the safe equivalent of the C++
//!    nullable `Error*`).
//!
//! The cross-platform mapping follows the C++ source:
//! - `Pcsx2Error::Errno` formats via `libc::strerror_r` (POSIX) /
//!   `strerror` (Windows CRT).
//! - `Pcsx2Error::Socket` is `Win32` on Windows, `Errno` everywhere else,
//!   exactly like the C++ `SetSocket` switch.
//! - `Pcsx2Error::Win32` / `Pcsx2Error::HResult` resolve through the
//!   Win32 `FormatMessageW` API; on non-Windows they fall back to a
//!   hex-coded string. They are *not* `cfg`-gated because we want the
//!   Rust enum to be the same shape on every platform, and a future
//!   cross-platform toolchain can still hand us Win32 codes.
//!
//! ## FFI
//!
//! The C++ side keeps owning `Error` objects. The FFI exports below
//! allocate and free Rust-side `Error` instances behind opaque pointers
//! using `Box::into_raw` / `Box::from_raw`. Each export is `unsafe` in
//! the sense that the C++ caller must honour the ownership contract:
//!
//! - One `pcsx2_error_destroy` per `pcsx2_error_create`.
//! - String/message buffers must be sized as documented.
//! - `*mut Error` pointers handed to any other export must either be null
//!   or have been produced by `pcsx2_error_create` on this Rust side.
//!
//! A separate `Pcsx2ErrorC` POD struct (`ty: u32`, `message: [u8; 1024]`)
//! is exposed for the value-passing C ABI used by PCSX2's auto-generated
//! headers.

#![allow(clippy::needless_pass_by_value)]

use std::ffi::{CStr, CString};
use std::fmt;
use std::ptr;

use libc::{c_char, c_int, c_uint};

// ---------------------------------------------------------------------------
// Core enum: `Pcsx2Error`
// ---------------------------------------------------------------------------

/// The category of a PCSX2 error plus its native payload.
///
/// `None` carries no data; the other variants carry either a numeric
/// error code (as reported by the OS / API) or a user-supplied string.
/// The string variant owns its message so that lifetimes are trivial
/// — the C++ `std::string` semantics map cleanly to `String`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pcsx2Error {
    /// No error — sentinel value, equivalent to the C++ `Type::None`.
    None,
    /// `errno` value from a POSIX/libc call. `i32` so it matches the
    /// width of `errno` on every supported platform.
    Errno(i32),
    /// Socket error code. On Windows this is a Win32 `WSA*` code, on
    /// POSIX platforms it is just `errno`.
    Socket(i32),
    /// User-supplied string error. Always carries the message verbatim
    /// so it round-trips through `Display` without loss.
    User(String),
    /// Win32 error code as returned by `GetLastError()` or one of the
    /// `Reg*` / `FormatMessage`-style APIs. Held as `u32` (the width of
    /// `DWORD`).
    Win32(u32),
    /// COM `HRESULT` (the sign-extended `long` from C++). Held as
    /// `i32` to round-trip through the C ABI without truncation.
    HResult(i32),
}

impl Pcsx2Error {
    /// Return the kind tag for this error, mirroring C++ `Error::GetType()`.
    ///
    /// The discriminant values match the C++ enum exactly so they can be
    /// passed back to C++ without translation.
    #[inline]
    pub fn kind(&self) -> ErrorKind {
        match self {
            Pcsx2Error::None => ErrorKind::None,
            Pcsx2Error::Errno(_) => ErrorKind::Errno,
            Pcsx2Error::Socket(_) => ErrorKind::Socket,
            Pcsx2Error::User(_) => ErrorKind::User,
            Pcsx2Error::Win32(_) => ErrorKind::Win32,
            Pcsx2Error::HResult(_) => ErrorKind::HResult,
        }
    }

    /// `true` if this is anything other than `None`.
    #[inline]
    pub fn is_valid(&self) -> bool {
        !matches!(self, Pcsx2Error::None)
    }

    /// Return a fresh `String` description. Mirrors
    /// `Error::GetDescription()` from the C++ class — but on `Pcsx2Error`
    /// itself so it can be used as a `std::error::Error` payload without
    /// going through `Error`.
    pub fn description(&self) -> String {
        match self {
            Pcsx2Error::None => String::new(),
            Pcsx2Error::Errno(e) => format_errno_message("", *e),
            Pcsx2Error::Socket(e) => format_socket_message("", *e),
            Pcsx2Error::User(s) => s.clone(),
            Pcsx2Error::Win32(e) => format_win32_message("", *e),
            Pcsx2Error::HResult(e) => format_hresult_message("", *e),
        }
    }

    /// As [`description`](Self::description), but with a caller-supplied
    /// prefix prepended. Mirrors the C++ `set_*(*, prefix, code)` overloads.
    pub fn description_with_prefix(&self, prefix: &str) -> String {
        match self {
            Pcsx2Error::None => String::new(),
            Pcsx2Error::Errno(e) => format_errno_message(prefix, *e),
            Pcsx2Error::Socket(e) => format_socket_message(prefix, *e),
            Pcsx2Error::User(s) => {
                if prefix.is_empty() {
                    s.clone()
                } else {
                    let mut buf = String::with_capacity(prefix.len() + s.len());
                    buf.push_str(prefix);
                    buf.push_str(s);
                    buf
                }
            }
            Pcsx2Error::Win32(e) => format_win32_message(prefix, *e),
            Pcsx2Error::HResult(e) => format_hresult_message(prefix, *e),
        }
    }
}

impl fmt::Display for Pcsx2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description())
    }
}

impl std::error::Error for Pcsx2Error {}

impl Default for Pcsx2Error {
    #[inline]
    fn default() -> Self {
        Pcsx2Error::None
    }
}

impl From<&str> for Pcsx2Error {
    #[inline]
    fn from(s: &str) -> Self {
        Pcsx2Error::User(s.to_owned())
    }
}

impl From<String> for Pcsx2Error {
    #[inline]
    fn from(s: String) -> Self {
        Pcsx2Error::User(s)
    }
}

// ---------------------------------------------------------------------------
// Kind tag — 1:1 with the C++ `enum class Type`.
// ---------------------------------------------------------------------------

/// Numeric tag corresponding to the C++ `Error::Type` enumerators.
///
/// Values are stable across the FFI boundary and must match the C++ side.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    None = 0,
    Errno = 1,
    Socket = 2,
    User = 3,
    Win32 = 4,
    HResult = 5,
}

// ---------------------------------------------------------------------------
// Error wrapper — 1:1 with the C++ `class Error`.
// ---------------------------------------------------------------------------

/// Mutable error container. The closest Rust equivalent to the C++
/// `class Error` — copy/move semantics, an `is_valid()` flag, mutating
/// `set_*` methods, and prefix/suffix decoration.
///
/// Internally `Error` caches the formatted description string so that
/// [`get_description`](Self::get_description) is allocation-free and so
/// that [`add_prefix`](Self::add_prefix) / [`add_suffix`](Self::add_suffix)
/// can mutate the cached text directly. The cache is rebuilt every
/// time a `set_*` method is called.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Error {
    inner: Pcsx2Error,
    /// Cached description, kept in sync with `inner` by every `set_*`.
    /// Empty when `inner == Pcsx2Error::None`.
    description: String,
}

impl Error {
    /// Construct a fresh, "no error" container.
    #[inline]
    pub fn new() -> Self {
        Self { inner: Pcsx2Error::None, description: String::new() }
    }

    /// Wrap an existing [`Pcsx2Error`] value, materialising its
    /// description eagerly.
    pub fn from_pcsx2(inner: Pcsx2Error) -> Self {
        let description = inner.description();
        Self { inner, description }
    }

    /// Borrow the inner [`Pcsx2Error`] payload.
    #[inline]
    pub fn inner(&self) -> &Pcsx2Error {
        &self.inner
    }

    /// Consume the wrapper and return the inner [`Pcsx2Error`].
    #[inline]
    pub fn into_inner(self) -> Pcsx2Error {
        self.inner
    }

    /// Mirror of C++ `Error::GetType()`.
    #[inline]
    pub fn get_type(&self) -> ErrorKind {
        self.inner.kind()
    }

    /// Mirror of C++ `Error::IsValid()`.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.inner.is_valid()
    }

    /// Mirror of C++ `Error::GetDescription()`.
    #[inline]
    pub fn get_description(&self) -> &str {
        &self.description
    }

    /// Reset the error to `None`. Both the type tag and the cached
    /// description string are cleared.
    pub fn clear(&mut self) {
        self.inner = Pcsx2Error::None;
        self.description.clear();
    }

    // -- set_* methods ----------------------------------------------------

    /// Mirror of C++ `Error::SetErrno(int)`.
    pub fn set_errno(&mut self, err: i32) {
        self.inner = Pcsx2Error::Errno(err);
        self.description = format_errno_message("", err);
    }

    /// Mirror of C++ `Error::SetErrno(string_view, int)`.
    pub fn set_errno_with_prefix(&mut self, prefix: &str, err: i32) {
        self.inner = Pcsx2Error::Errno(err);
        self.description = format_errno_message(prefix, err);
    }

    /// Mirror of C++ `Error::SetSocket(int)`.
    pub fn set_socket(&mut self, err: i32) {
        self.inner = Pcsx2Error::Socket(err);
        self.description = format_socket_message("", err);
    }

    /// Mirror of C++ `Error::SetSocket(string_view, int)`.
    pub fn set_socket_with_prefix(&mut self, prefix: &str, err: i32) {
        self.inner = Pcsx2Error::Socket(err);
        self.description = format_socket_message(prefix, err);
    }

    /// Mirror of C++ `Error::SetString(std::string)`.
    pub fn set_string(&mut self, description: impl Into<String>) {
        let s: String = description.into();
        self.description = s.clone();
        self.inner = Pcsx2Error::User(s);
    }

    /// Mirror of C++ `Error::SetStringView(std::string_view)`.
    pub fn set_string_view(&mut self, description: &str) {
        self.description = description.to_owned();
        self.inner = Pcsx2Error::User(self.description.clone());
    }

    /// Set a `Win32` error. Available on every platform so the enum
    /// shape is consistent; on non-Windows the description is a
    /// best-effort hex placeholder, mirroring the C++ "could not
    /// resolve" branch.
    pub fn set_win32(&mut self, err: u32) {
        self.inner = Pcsx2Error::Win32(err);
        self.description = format_win32_message("", err);
    }

    pub fn set_win32_with_prefix(&mut self, prefix: &str, err: u32) {
        self.inner = Pcsx2Error::Win32(err);
        self.description = format_win32_message(prefix, err);
    }

    /// Set an `HResult` error. Same cross-platform story as
    /// [`set_win32`](Self::set_win32).
    pub fn set_hresult(&mut self, err: i32) {
        self.inner = Pcsx2Error::HResult(err);
        self.description = format_hresult_message("", err);
    }

    pub fn set_hresult_with_prefix(&mut self, prefix: &str, err: i32) {
        self.inner = Pcsx2Error::HResult(err);
        self.description = format_hresult_message(prefix, err);
    }

    // -- prefix/suffix ----------------------------------------------------

    /// Prepend `prefix` to the existing description. Mirrors C++
    /// `Error::AddPrefix`.
    pub fn add_prefix(&mut self, prefix: &str) {
        if !self.is_valid() {
            return;
        }
        if prefix.is_empty() {
            return;
        }
        let mut combined = String::with_capacity(prefix.len() + self.description.len());
        combined.push_str(prefix);
        combined.push_str(&self.description);
        self.description = combined;
        // Upgrade to `User` so the new description is preserved exactly
        // (the numeric code can't be re-derived from the prefix anyway,
        // and this matches how `add_prefix` would have behaved on the
        // C++ side once a non-user variant had been wrapped).
        self.inner = Pcsx2Error::User(self.description.clone());
    }

    /// Append `suffix` to the existing description. Mirrors C++
    /// `Error::AddSuffix`.
    pub fn add_suffix(&mut self, suffix: &str) {
        if !self.is_valid() {
            return;
        }
        if suffix.is_empty() {
            return;
        }
        self.description.push_str(suffix);
        self.inner = Pcsx2Error::User(self.description.clone());
    }

    // -- static helpers mirroring the C++ `Error::Set*(Error*, ...)` API --

    /// C++ `Error::SetErrno(Error*, int)` — only writes when `target`
    /// is `Some`. Lets callers write `Error::set_errno_opt(err.as_mut(), e)`
    /// instead of `if let Some(t) = err { t.set_errno(e); }`.
    #[inline]
    pub fn set_errno_opt(target: Option<&mut Error>, err: i32) {
        if let Some(t) = target {
            t.set_errno(err);
        }
    }

    #[inline]
    pub fn set_errno_opt_prefix(target: Option<&mut Error>, prefix: &str, err: i32) {
        if let Some(t) = target {
            t.set_errno_with_prefix(prefix, err);
        }
    }

    #[inline]
    pub fn set_socket_opt(target: Option<&mut Error>, err: i32) {
        if let Some(t) = target {
            t.set_socket(err);
        }
    }

    #[inline]
    pub fn set_socket_opt_prefix(target: Option<&mut Error>, prefix: &str, err: i32) {
        if let Some(t) = target {
            t.set_socket_with_prefix(prefix, err);
        }
    }

    #[inline]
    pub fn set_string_opt(target: Option<&mut Error>, description: impl Into<String>) {
        if let Some(t) = target {
            t.set_string(description);
        }
    }

    #[inline]
    pub fn set_string_opt_view(target: Option<&mut Error>, description: &str) {
        if let Some(t) = target {
            t.set_string_view(description);
        }
    }

    #[inline]
    pub fn set_win32_opt(target: Option<&mut Error>, err: u32) {
        if let Some(t) = target {
            t.set_win32(err);
        }
    }

    #[inline]
    pub fn set_hresult_opt(target: Option<&mut Error>, err: i32) {
        if let Some(t) = target {
            t.set_hresult(err);
        }
    }

    /// C++ `Error::Clear(Error*)`.
    #[inline]
    pub fn clear_opt(target: Option<&mut Error>) {
        if let Some(t) = target {
            t.clear();
        }
    }

    /// C++ `Error::AddPrefix(Error*, string_view)`.
    #[inline]
    pub fn add_prefix_opt(target: Option<&mut Error>, prefix: &str) {
        if let Some(t) = target {
            t.add_prefix(prefix);
        }
    }

    /// C++ `Error::AddSuffix(Error*, string_view)`.
    #[inline]
    pub fn add_suffix_opt(target: Option<&mut Error>, suffix: &str) {
        if let Some(t) = target {
            t.add_suffix(suffix);
        }
    }

    // -- factory helpers (C++ `Create*`) ---------------------------------

    /// C++ `Error::CreateNone()`.
    #[inline]
    pub fn create_none() -> Self {
        Self::new()
    }

    /// C++ `Error::CreateErrno(int)`.
    pub fn create_errno(err: i32) -> Self {
        let mut e = Self::new();
        e.set_errno(err);
        e
    }

    /// C++ `Error::CreateSocket(int)`.
    pub fn create_socket(err: i32) -> Self {
        let mut e = Self::new();
        e.set_socket(err);
        e
    }

    /// C++ `Error::CreateString(std::string)`.
    pub fn create_string(description: impl Into<String>) -> Self {
        let mut e = Self::new();
        e.set_string(description);
        e
    }

    /// C++ `Error::CreateWin32(unsigned long)`.
    pub fn create_win32(err: u32) -> Self {
        let mut e = Self::new();
        e.set_win32(err);
        e
    }

    /// C++ `Error::CreateHResult(long)`.
    pub fn create_hresult(err: i32) -> Self {
        let mut e = Self::new();
        e.set_hresult(err);
        e
    }
}

impl From<Pcsx2Error> for Error {
    #[inline]
    fn from(inner: Pcsx2Error) -> Self {
        Self::from_pcsx2(inner)
    }
}

impl From<Error> for Pcsx2Error {
    #[inline]
    fn from(e: Error) -> Self {
        e.inner
    }
}

impl From<&str> for Error {
    #[inline]
    fn from(s: &str) -> Self {
        Self::create_string(s)
    }
}

impl From<String> for Error {
    #[inline]
    fn from(s: String) -> Self {
        Self::create_string(s)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description)
    }
}

impl std::error::Error for Error {}

impl PartialEq<Pcsx2Error> for Error {
    #[inline]
    fn eq(&self, other: &Pcsx2Error) -> bool {
        &self.inner == other
    }
}

impl PartialEq<Error> for Pcsx2Error {
    #[inline]
    fn eq(&self, other: &Error) -> bool {
        self == &other.inner
    }
}

// ---------------------------------------------------------------------------
// Message formatting helpers
// ---------------------------------------------------------------------------
//
// The C++ side calls into Win32 / POSIX to turn an error code into a
// readable message. We do the same, but isolate the formatting logic
// from the type wrappers so it's easy to test in isolation.

fn format_errno_message(prefix: &str, err: i32) -> String {
    let body = strerror_message(err);
    if prefix.is_empty() {
        format!("errno {err}: {body}")
    } else {
        format!("{prefix}errno {err}: {body}")
    }
}

fn format_socket_message(prefix: &str, err: i32) -> String {
    // Mirror C++: on Windows, socket errors *are* Win32 errors; on
    // POSIX they're the same as errno. We pick the formatter at
    // runtime via the same `cfg!` switch.
    #[cfg(windows)]
    let body = format_win32_body(err as u32);
    #[cfg(not(windows))]
    let body = strerror_message(err);
    if prefix.is_empty() {
        format!("socket error {err}: {body}")
    } else {
        format!("{prefix}socket error {err}: {body}")
    }
}

fn format_win32_message(prefix: &str, err: u32) -> String {
    let body = format_win32_body(err);
    if prefix.is_empty() {
        format!("Win32 Error {err}: {body}")
    } else {
        format!("{prefix}Win32 Error {err}: {body}")
    }
}

fn format_hresult_message(prefix: &str, err: i32) -> String {
    let body = format_win32_body(err as u32);
    let hex = format!("{:08X}", err as u32);
    if prefix.is_empty() {
        format!("HRESULT {hex}: {body}")
    } else {
        format!("{prefix}HRESULT {hex}: {body}")
    }
}

/// Best-effort string for a Win32 error code.
///
/// On Windows this calls `FormatMessageW`; on non-Windows we return
/// the same "<Could not resolve system error ID>" placeholder the
/// C++ source falls back to when the lookup fails.
fn format_win32_body(err: u32) -> String {
    #[cfg(windows)]
    {
        format_win32_body_windows(err)
    }
    #[cfg(not(windows))]
    {
        let _ = err;
        String::from("<Could not resolve system error ID>")
    }
}

#[cfg(windows)]
fn format_win32_body_windows(err: u32) -> String {
    // We deliberately do *not* depend on `windows-sys` / `winapi` in
    // this crate's `Cargo.toml`. To keep the code self-contained we
    // declare just the `FormatMessageW` entry point via raw FFI.
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;

    const FORMAT_MESSAGE_FROM_SYSTEM: u32 = 0x00001000;
    const FORMAT_MESSAGE_IGNORE_INSERTS: u32 = 0x00000200;

    extern "system" {
        fn FormatMessageW(
            dwFlags: u32,
            lpSource: *const core::ffi::c_void,
            dwMessageId: u32,
            dwLanguageId: u32,
            lpBuffer: *mut u16,
            nSize: u32,
            Arguments: *const core::ffi::c_void,
        ) -> u32;
    }

    let mut buf = [0u16; 512];
    let flags = FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS;
    // SAFETY: all arguments are POD or owned buffers; the API is
    // well-defined for `FORMAT_MESSAGE_FROM_SYSTEM | IGNORE_INSERTS`.
    let len = unsafe {
        FormatMessageW(
            flags,
            core::ptr::null(),
            err,
            0,
            buf.as_mut_ptr(),
            buf.len() as u32,
            core::ptr::null(),
        )
    };
    if len == 0 {
        return String::from("<Could not resolve system error ID>");
    }
    // Trim trailing whitespace, like the C++ loop.
    let mut end = len as usize;
    while end > 0 {
        let c = buf[end - 1];
        if c == u16::from(b' ')
            || c == u16::from(b'\t')
            || c == u16::from(b'\n')
            || c == u16::from(b'\r')
        {
            end -= 1;
        } else {
            break;
        }
    }
    OsString::from_wide(&buf[..end]).to_string_lossy().into_owned()
}

/// Wrapper around `strerror_r` / `strerror` that always returns a
/// `String`. Mirrors the C++ `strerror_s` / `strerror` switch: on
/// success we get the platform message, on failure we return a
/// placeholder so the resulting `Error` is never empty.
fn strerror_message(err: i32) -> String {
    #[cfg(target_os = "linux")]
    {
        // GNU `strerror_r` may return either a pointer into a
        // user-supplied buffer (when that buffer is large enough) or
        // a static string. POSIX `strerror_r` returns an int. The
        // `libc` crate's bindings resolve to whichever variant glibc
        // provides; on Linux we treat any non-null pointer as success.
        let mut buf = [0u8; 256];
        // SAFETY: buf is a valid stack buffer of sufficient size.
        let ret = unsafe { libc::strerror_r(err, buf.as_mut_ptr() as *mut _, buf.len()) };
        if ret != 0 {
            return String::from("<Could not get error message>");
        }
        // SAFETY: when ret == 0, buf contains the NUL-terminated error string.
        let raw = unsafe { CStr::from_ptr(buf.as_ptr() as *const std::ffi::c_char) }.to_string_lossy().into_owned();
        if raw.is_empty() {
            String::from("<Could not get error message>")
        } else {
            raw
        }
    }
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    {
        // XSI `strerror_r` returns int 0 on success, sets errno on failure.
        let mut buf = [0u8; 256];
        // SAFETY: buf is a valid stack buffer of sufficient size.
        let rc = unsafe { libc::strerror_r(err, buf.as_mut_ptr() as *mut _, buf.len()) };
        if rc != 0 {
            return String::from("<Could not get error message>");
        }
        // SAFETY: rc == 0 means buf holds a NUL-terminated C string.
        let raw = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) }
            .to_string_lossy()
            .into_owned();
        if raw.is_empty() {
            String::from("<Could not get error message>")
        } else {
            raw
        }
    }
    #[cfg(target_os = "windows")]
    {
        // MSVC's `strerror` is the only portable option on Windows.
        let p = unsafe { libc_strerror(err) };
        if p.is_null() {
            String::from("<Could not get error message>")
        } else {
            // SAFETY: p points at a NUL-terminated C string.
            let raw = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
            if raw.is_empty() {
                String::from("<Could not get error message>")
            } else {
                raw
            }
        }
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "windows"
    )))]
    {
        let _ = err;
        String::from("<Could not get error message>")
    }
}

// MSVC's `strerror` is hidden behind deprecation in modern Windows
// SDKs; declare it directly so we can call into the C runtime
// without going through `libc` (which doesn't re-export it).
#[cfg(target_os = "windows")]
extern "C" {
    #[link_name = "strerror"]
    fn libc_strerror(errnum: c_int) -> *mut c_char;
}

// ---------------------------------------------------------------------------
// FFI surface
// ---------------------------------------------------------------------------
//
// C++ owns all `*mut Error` pointers. The Rust side never frees a
// pointer it didn't produce, and the C++ side never frees one it
// didn't allocate. The functions below are the entire API; they
// match what PCSX2's auto-generated C header expects.

// =========================================================================
// POD payload: `Pcsx2ErrorC`
// =========================================================================

/// Value-type C ABI for passing an error across the boundary without
/// heap-allocating. Equivalent to the C++ `Error` *value* with the
/// description truncated to 1024 bytes.
///
/// Layout is `#[repr(C)]` and must not change without updating the
/// matching `pcsx2_common_rs.h` C header.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Pcsx2ErrorC {
    /// Discriminant matching [`ErrorKind`].
    pub ty: u32,
    /// NUL-terminated UTF-8 message. Unused bytes past the message are
    /// zeroed by construction.
    pub message: [u8; 1024],
}

impl Pcsx2ErrorC {
    /// Construct an "empty" C payload: `ty = 0` (None), all-zero message.
    pub const fn empty() -> Self {
        Self { ty: ErrorKind::None as u32, message: [0u8; 1024] }
    }

    /// Materialise a C payload from a Rust [`Pcsx2Error`]. The
    /// description is truncated to fit in the 1024-byte buffer.
    pub fn from_pcsx2(err: &Pcsx2Error) -> Self {
        let mut out = Self::empty();
        out.ty = err.kind() as u32;
        let desc = err.description();
        write_cstr(&mut out.message, &desc);
        out
    }

    /// Materialise a C payload from a Rust [`Error`].
    pub fn from_error(err: &Error) -> Self {
        let mut out = Self::empty();
        out.ty = err.get_type() as u32;
        write_cstr(&mut out.message, err.get_description());
        out
    }
}

impl Default for Pcsx2ErrorC {
    #[inline]
    fn default() -> Self {
        Self::empty()
    }
}

/// Copy `src` into `dst` as a NUL-terminated C string, truncating if
/// it does not fit. Always writes a trailing NUL.
fn write_cstr(dst: &mut [u8], src: &str) {
    // Cap so we always have room for the NUL byte.
    let max = dst.len().saturating_sub(1);
    let bytes = src.as_bytes();
    let n = bytes.len().min(max);
    dst[..n].copy_from_slice(&bytes[..n]);
    // Zero the rest of the buffer so the C side never observes stale
    // bytes from a previous use.
    for b in &mut dst[n..] {
        *b = 0;
    }
    // Place the terminating NUL. We've already ensured room above.
    dst[n] = 0;
}

// =========================================================================
// Opaque handle API
// =========================================================================

/// Allocate a fresh, empty `Error` on the heap and return an opaque
/// owning pointer. The C++ side stores this as `Error*`.
///
/// The caller must call [`pcsx2_error_destroy`] exactly once on the
/// returned pointer; double-free or use-after-free are the caller's
/// responsibility.
#[no_mangle]
pub extern "C" fn pcsx2_error_create() -> *mut Error {
    Box::into_raw(Box::new(Error::new()))
}

/// Free an `Error` previously allocated by [`pcsx2_error_create`].
///
/// Passing a null pointer is a no-op (matches `free`). After this
/// returns, `err` must not be used again.
#[no_mangle]
pub extern "C" fn pcsx2_error_destroy(err: *mut Error) {
    if err.is_null() {
        return;
    }
    // SAFETY: caller guarantees `err` was returned by
    // `pcsx2_error_create` and has not been freed.
    unsafe {
        drop(Box::from_raw(err));
    }
}

/// Mirror of C++ `Error::SetErrno`. Silently no-ops on a null pointer.
#[no_mangle]
pub extern "C" fn pcsx2_error_set_errno(err: *mut Error, code: c_int) {
    if err.is_null() {
        return;
    }
    // SAFETY: caller guarantees `err` is a live, owned `Error*`.
    let e: &mut Error = unsafe { &mut *err };
    e.set_errno(code);
}

/// Mirror of C++ `Error::SetString`. `msg` must be a NUL-terminated
/// C string or null. A null `msg` produces an empty user error.
#[no_mangle]
pub extern "C" fn pcsx2_error_set_string(err: *mut Error, msg: *const c_char) {
    if err.is_null() {
        return;
    }
    // SAFETY: caller guarantees ownership of `err` and that `msg` is
    // either null or a NUL-terminated C string.
    let s: String = if msg.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(msg) }
            .to_string_lossy()
            .into_owned()
    };
    let e: &mut Error = unsafe { &mut *err };
    e.set_string(s);
}

/// Copy the description into the caller's `out` buffer.
///
/// `out_len` is the size of `out` in bytes. The returned `u32` is the
/// number of bytes that *would* have been written (excluding the
/// trailing NUL); if it is greater than `out_len`, the result was
/// truncated and the caller should call again with a larger buffer.
///
/// A null `err` always writes an empty NUL-terminated string and
/// returns 0. A null `out` returns 0 without writing.
#[no_mangle]
pub extern "C" fn pcsx2_error_get_message(
    err: *mut Error,
    out: *mut c_char,
    out_len: c_uint,
) -> c_uint {
    if err.is_null() {
        if !out.is_null() && out_len > 0 {
            // SAFETY: caller guarantees `out` points to a writable
            // buffer of `out_len` bytes.
            unsafe {
                ptr::write_bytes(out, 0, out_len as usize);
            }
        }
        return 0;
    }

    // SAFETY: caller guarantees ownership of `err`.
    let desc = unsafe { &*err }.get_description();
    let needed = desc.len() as c_uint;

    if out.is_null() || out_len == 0 {
        return needed;
    }

    // SAFETY: caller guarantees `out` is writable for `out_len` bytes.
    let buf = unsafe { std::slice::from_raw_parts_mut(out as *mut u8, out_len as usize) };
    // Leave room for a NUL terminator.
    let cap = (out_len as usize).saturating_sub(1);
    let n = desc.len().min(cap);
    buf[..n].copy_from_slice(&desc.as_bytes()[..n]);
    buf[n] = 0;
    needed
}

/// Mirror of C++ `Error::IsValid`. A null pointer returns `false`.
#[no_mangle]
pub extern "C" fn pcsx2_error_is_valid(err: *mut Error) -> bool {
    if err.is_null() {
        return false;
    }
    // SAFETY: caller guarantees ownership of `err`.
    unsafe { &*err }.is_valid()
}

/// Mirror of C++ `Error::Clear`. A null pointer is a no-op.
#[no_mangle]
pub extern "C" fn pcsx2_error_clear(err: *mut Error) {
    if err.is_null() {
        return;
    }
    // SAFETY: caller guarantees ownership of `err`.
    let e: &mut Error = unsafe { &mut *err };
    e.clear();
}

/// Mirror of C++ `Error::SetWin32(unsigned long)` (instance method).
/// A null pointer is a no-op. `code` is the Win32 `DWORD` error code
/// (e.g. returned by `GetLastError()`).
#[no_mangle]
pub extern "C" fn pcsx2_error_set_win32_inst(err: *mut Error, code: c_uint) {
    if err.is_null() {
        return;
    }
    // SAFETY: caller guarantees ownership of `err`.
    let e: &mut Error = unsafe { &mut *err };
    e.set_win32(code);
}

/// Mirror of C++ `Error::SetWin32(Error*, unsigned long)` (static).
/// A null pointer is a no-op. `code` is the Win32 `DWORD` error code.
#[no_mangle]
pub extern "C" fn pcsx2_error_set_win32_code(err: *mut Error, code: c_uint) {
    if err.is_null() {
        return;
    }
    // SAFETY: caller guarantees ownership of `err`.
    let e: &mut Error = unsafe { &mut *err };
    e.set_win32(code);
}

/// Mirror of C++ `Error::SetWin32(Error*, std::string_view, unsigned long)`
/// (static with prefix). `description` must be a NUL-terminated C string
/// or null. A null `description` is treated as an empty prefix.
#[no_mangle]
pub extern "C" fn pcsx2_error_set_win32_static(
    err: *mut Error,
    description: *const c_char,
    code: c_uint,
) {
    if err.is_null() {
        return;
    }
    // SAFETY: caller guarantees ownership of `err` and that `description`
    // is either null or a NUL-terminated C string.
    let s: String = if description.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(description) }
            .to_string_lossy()
            .into_owned()
    };
    let e: &mut Error = unsafe { &mut *err };
    e.set_win32_with_prefix(&s, code);
}

/// Mirror of C++ `Error::SetHResult(long)` (instance method). A null
/// pointer is a no-op. `code` is a COM `HRESULT` (sign-extended 32-bit
/// value).
#[no_mangle]
pub extern "C" fn pcsx2_error_set_hresult_inst(err: *mut Error, code: c_int) {
    if err.is_null() {
        return;
    }
    // SAFETY: caller guarantees ownership of `err`.
    let e: &mut Error = unsafe { &mut *err };
    e.set_hresult(code);
}

/// Mirror of C++ `Error::SetHResult(Error*, std::string_view, long)`
/// (static with description). `description` must be a NUL-terminated C
/// string or null. A null `description` is treated as an empty prefix.
#[no_mangle]
pub extern "C" fn pcsx2_error_set_hresult(
    err: *mut Error,
    description: *const c_char,
    hr: c_int,
) {
    if err.is_null() {
        return;
    }
    // SAFETY: caller guarantees ownership of `err` and that `description`
    // is either null or a NUL-terminated C string.
    let s: String = if description.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(description) }
            .to_string_lossy()
            .into_owned()
    };
    let e: &mut Error = unsafe { &mut *err };
    e.set_hresult_with_prefix(&s, hr);
}

/// Mirror of C++ `Error::SetWin32(Error*, string_view, unsigned long)`.
/// `prefix` must be a NUL-terminated C string or null. A null `prefix`
/// is treated as an empty prefix.
#[no_mangle]
pub extern "C" fn pcsx2_error_set_win32_prefix(
    err: *mut Error,
    prefix: *const c_char,
    code: c_uint,
) {
    if err.is_null() {
        return;
    }
    // SAFETY: caller guarantees ownership of `err` and that `prefix` is
    // either null or a NUL-terminated C string.
    let s: String = if prefix.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(prefix) }
            .to_string_lossy()
            .into_owned()
    };
    let e: &mut Error = unsafe { &mut *err };
    e.set_win32_with_prefix(&s, code);
}

/// Mirror of C++ `Error::SetHResult(Error*, string_view, long)`.
/// `prefix` must be a NUL-terminated C string or null. A null `prefix`
/// is treated as an empty prefix.
#[no_mangle]
pub extern "C" fn pcsx2_error_set_hresult_prefix(
    err: *mut Error,
    prefix: *const c_char,
    code: c_int,
) {
    if err.is_null() {
        return;
    }
    // SAFETY: caller guarantees ownership of `err` and that `prefix` is
    // either null or a NUL-terminated C string.
    let s: String = if prefix.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(prefix) }
            .to_string_lossy()
            .into_owned()
    };
    let e: &mut Error = unsafe { &mut *err };
    e.set_hresult_with_prefix(&s, code);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_is_none() {
        let e = Error::new();
        assert!(!e.is_valid());
        assert_eq!(e.get_type(), ErrorKind::None);
        assert_eq!(e.inner(), &Pcsx2Error::None);
        assert_eq!(e.get_description(), "");
    }

    #[test]
    fn default_matches_new() {
        let a = Error::default();
        let b = Error::new();
        assert_eq!(a, b);
    }

    #[test]
    fn set_errno_makes_valid() {
        let mut e = Error::new();
        e.set_errno(2);
        assert!(e.is_valid());
        // Type tag is preserved as Errno.
        assert_eq!(e.get_type(), ErrorKind::Errno);
        let desc = e.get_description();
        assert!(desc.contains("errno 2"), "desc was {desc:?}");
    }

    #[test]
    fn set_string_round_trips() {
        let mut e = Error::new();
        e.set_string("oops");
        assert!(e.is_valid());
        assert_eq!(e.get_type(), ErrorKind::User);
        assert_eq!(e.get_description(), "oops");
    }

    #[test]
    fn clear_returns_to_none() {
        let mut e = Error::create_string("boom");
        assert!(e.is_valid());
        e.clear();
        assert!(!e.is_valid());
        assert_eq!(e.get_type(), ErrorKind::None);
        assert_eq!(e.get_description(), "");
    }

    #[test]
    fn prefix_extends_user() {
        let mut e = Error::create_string("inner");
        e.add_prefix("prefix:");
        assert_eq!(e.get_description(), "prefix:inner");
    }

    #[test]
    fn suffix_extends_user() {
        let mut e = Error::create_string("inner");
        e.add_suffix(":suffix");
        assert_eq!(e.get_description(), "inner:suffix");
    }

    #[test]
    fn prefix_on_none_is_noop() {
        let mut e = Error::new();
        e.add_prefix("anything");
        assert!(!e.is_valid());
    }

    #[test]
    fn opt_helpers_ignore_none() {
        // Option::None should be silently ignored — equivalent to a
        // null pointer in C++.
        Error::set_errno_opt(None, 1);
        Error::set_string_opt(None, "x");
        Error::clear_opt(None);
    }

    #[test]
    fn opt_helpers_write_through_some() {
        let mut e = Error::new();
        Error::set_string_opt(Some(&mut e), "hello");
        assert!(e.is_valid());
        assert_eq!(e.get_description(), "hello");
    }

    #[test]
    fn create_helpers_match_set() {
        let a = Error::create_errno(7);
        let mut b = Error::new();
        b.set_errno(7);
        assert_eq!(a, b);

        let a = Error::create_string("xyz");
        let mut b = Error::new();
        b.set_string("xyz");
        assert_eq!(a, b);
    }

    #[test]
    fn equality_is_structural() {
        let a = Error::create_string("same");
        let b = Error::create_string("same");
        let c = Error::create_string("different");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn from_conversions_round_trip() {
        let raw = Pcsx2Error::User("payload".into());
        let e: Error = raw.clone().into();
        assert_eq!(e.inner(), &raw);
        let back: Pcsx2Error = e.into();
        assert_eq!(back, raw);
    }

    #[test]
    fn pcsx2_error_c_truncates_long_messages() {
        let mut e = Error::new();
        let big = "a".repeat(2048);
        e.set_string(&big);
        let c = Pcsx2ErrorC::from_error(&e);
        assert_eq!(c.ty, ErrorKind::User as u32);
        // 1023 bytes of 'a' plus a NUL terminator at index 1023.
        assert_eq!(c.message[1023], 0);
        assert!(c.message[..1023].iter().all(|&b| b == b'a'));
    }

    #[test]
    fn pcsx2_error_c_for_none_is_zeroed() {
        let c = Pcsx2ErrorC::from_error(&Error::new());
        assert_eq!(c.ty, 0);
        assert!(c.message.iter().all(|&b| b == 0));
    }

    // --- FFI round-trips ------------------------------------------------

    #[test]
    fn ffi_create_destroy_round_trip() {
        let p = pcsx2_error_create();
        assert!(!p.is_null());
        assert!(!pcsx2_error_is_valid(p));
        pcsx2_error_destroy(p);
    }

    #[test]
    fn ffi_set_errno_marks_valid() {
        let p = pcsx2_error_create();
        pcsx2_error_set_errno(p, 2);
        assert!(pcsx2_error_is_valid(p));
        pcsx2_error_destroy(p);
    }

    #[test]
    fn ffi_set_string_and_read_back() {
        let p = pcsx2_error_create();
        let msg = CString::new("from c").unwrap();
        pcsx2_error_set_string(p, msg.as_ptr());
        assert!(pcsx2_error_is_valid(p));

        let mut buf = [0u8; 64];
        let n = pcsx2_error_get_message(
            p,
            buf.as_mut_ptr() as *mut c_char,
            buf.len() as c_uint,
        );
        assert_eq!(n, "from c".len() as c_uint);
        // C string is NUL-terminated at the returned length.
        assert_eq!(buf[n as usize], 0);
        assert_eq!(&buf[..n as usize], b"from c");

        pcsx2_error_destroy(p);
    }

    #[test]
    fn ffi_clear_resets_to_invalid() {
        let p = pcsx2_error_create();
        pcsx2_error_set_errno(p, 1);
        assert!(pcsx2_error_is_valid(p));
        pcsx2_error_clear(p);
        assert!(!pcsx2_error_is_valid(p));
        pcsx2_error_destroy(p);
    }

    #[test]
    fn ffi_null_pointers_are_safe() {
        // Each call must accept null without UB.
        assert!(!pcsx2_error_is_valid(ptr::null_mut()));
        pcsx2_error_destroy(ptr::null_mut());
        pcsx2_error_set_errno(ptr::null_mut(), 1);
        pcsx2_error_set_string(ptr::null_mut(), ptr::null());
        pcsx2_error_set_win32_inst(ptr::null_mut(), 5);
        pcsx2_error_set_win32_code(ptr::null_mut(), 5);
        pcsx2_error_set_win32_static(ptr::null_mut(), ptr::null(), 5);
        pcsx2_error_set_hresult_inst(ptr::null_mut(), -1);
        pcsx2_error_set_hresult(ptr::null_mut(), ptr::null(), -1);
        pcsx2_error_set_win32_prefix(ptr::null_mut(), ptr::null(), 5);
        pcsx2_error_set_hresult_prefix(ptr::null_mut(), ptr::null(), -1);
        pcsx2_error_clear(ptr::null_mut());
        let n = pcsx2_error_get_message(ptr::null_mut(), ptr::null_mut(), 0);
        assert_eq!(n, 0);
    }

    #[test]
    fn ffi_get_message_reports_required_size() {
        let p = pcsx2_error_create();
        let big = "x".repeat(2000);
        let cstr = CString::new(big.clone()).unwrap();
        pcsx2_error_set_string(p, cstr.as_ptr());

        let mut tiny = [0u8; 16];
        let n = pcsx2_error_get_message(
            p,
            tiny.as_mut_ptr() as *mut c_char,
            tiny.len() as c_uint,
        );
        // The full string is 2000 bytes; we got back the size *needed*
        // and the buffer was truncated.
        assert_eq!(n, 2000);
        assert_eq!(tiny[15], 0);
        assert!(tiny[..15].iter().all(|&b| b == b'x'));

        // Now ask for the full size.
        let mut big_buf = vec![0u8; 2001];
        let n = pcsx2_error_get_message(
            p,
            big_buf.as_mut_ptr() as *mut c_char,
            big_buf.len() as c_uint,
        );
        assert_eq!(n, 2000);
        assert_eq!(&big_buf[..2000], big.as_bytes());

        pcsx2_error_destroy(p);
    }

    #[test]
    fn ffi_set_win32_marks_valid() {
        let p = pcsx2_error_create();
        assert!(!pcsx2_error_is_valid(p));
        pcsx2_error_set_win32_inst(p, 5);
        assert!(pcsx2_error_is_valid(p));
        pcsx2_error_destroy(p);
    }

    #[test]
    fn ffi_set_hresult_marks_valid() {
        let p = pcsx2_error_create();
        assert!(!pcsx2_error_is_valid(p));
        pcsx2_error_set_hresult_inst(p, -1);
        assert!(pcsx2_error_is_valid(p));
        pcsx2_error_destroy(p);
    }

    #[test]
    fn ffi_set_win32_code_marks_valid() {
        let p = pcsx2_error_create();
        assert!(!pcsx2_error_is_valid(p));
        pcsx2_error_set_win32_code(p, 5);
        assert!(pcsx2_error_is_valid(p));
        pcsx2_error_destroy(p);
    }

    #[test]
    fn ffi_set_hresult_static_marks_valid() {
        let p = pcsx2_error_create();
        assert!(!pcsx2_error_is_valid(p));
        // null description path
        pcsx2_error_set_hresult(p, ptr::null(), -1);
        assert!(pcsx2_error_is_valid(p));
        pcsx2_error_clear(p);
        assert!(!pcsx2_error_is_valid(p));
        // real prefix path
        let desc = CString::new("tag:").unwrap();
        pcsx2_error_set_hresult(p, desc.as_ptr(), -1);
        assert!(pcsx2_error_is_valid(p));
        pcsx2_error_destroy(p);
    }

    #[test]
    fn ffi_set_win32_static_marks_valid() {
        let p = pcsx2_error_create();
        assert!(!pcsx2_error_is_valid(p));
        // null description path
        pcsx2_error_set_win32_static(p, ptr::null(), 5);
        assert!(pcsx2_error_is_valid(p));
        pcsx2_error_clear(p);
        assert!(!pcsx2_error_is_valid(p));
        // real prefix path
        let desc = CString::new("tag:").unwrap();
        pcsx2_error_set_win32_static(p, desc.as_ptr(), 5);
        assert!(pcsx2_error_is_valid(p));
        pcsx2_error_destroy(p);
    }

    #[test]
    fn ffi_set_win32_prefix_marks_valid() {
        let p = pcsx2_error_create();
        assert!(!pcsx2_error_is_valid(p));

        // First, exercise the null-prefix path (should behave like an
        // empty prefix).
        pcsx2_error_set_win32_prefix(p, ptr::null(), 5);
        assert!(pcsx2_error_is_valid(p));

        // Then re-use the same handle with a real C-string prefix.
        pcsx2_error_clear(p);
        assert!(!pcsx2_error_is_valid(p));

        let prefix = CString::new("tag:").unwrap();
        pcsx2_error_set_win32_prefix(p, prefix.as_ptr(), 5);
        assert!(pcsx2_error_is_valid(p));

        pcsx2_error_destroy(p);
    }

    #[test]
    fn ffi_set_hresult_prefix_marks_valid() {
        let p = pcsx2_error_create();
        assert!(!pcsx2_error_is_valid(p));

        // First, exercise the null-prefix path.
        pcsx2_error_set_hresult_prefix(p, ptr::null(), -1);
        assert!(pcsx2_error_is_valid(p));

        // Then re-use the same handle with a real C-string prefix.
        pcsx2_error_clear(p);
        assert!(!pcsx2_error_is_valid(p));

        let prefix = CString::new("tag:").unwrap();
        pcsx2_error_set_hresult_prefix(p, prefix.as_ptr(), -1);
        assert!(pcsx2_error_is_valid(p));

        pcsx2_error_destroy(p);
    }
}