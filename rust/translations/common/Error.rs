// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Error handling primitives for PCSX2.
//!
//! This module is the Rust translation of the legacy `common/Error.{h,cpp}`
//! pair. It exposes:
//!
//! * [`Pcsx2Error`] — a lightweight wrapper around a human-readable message
//!   that implements [`std::error::Error`] and [`std::fmt::Display`].
//! * [`Error`] — an enum describing the origin/kind of an error (I/O,
//!   invalid parameter, out-of-memory, recoverable, etc.).
//! * [`Pcsx2Exception`] — a small struct that pairs a message with a captured
//!   backtrace, intended for unrecoverable failures.
//! * [`Result`] — a convenience alias mapping `Ok` / `Pcsx2Error`.
//!
//! Only `std` is depended upon. There are no platform-specific code paths
//! in this translation: the original `Win32` / `HResult` cases collapse into
//! the generic [`ErrorKind::Io`] / [`ErrorKind::User`] variants, which is
//! the idiomatic shape on stable Rust without an extra `winapi` crate.

use std::backtrace::{Backtrace, BacktraceStatus};
use std::error::Error as StdError;
use std::fmt;
use std::io;

/// Convenience alias for `Result<T, Pcsx2Error>` used throughout PCSX2.
pub type Result<T> = std::result::Result<T, Pcsx2Error>;

/// Categorises the source/origin of a [`Pcsx2Error`].
///
/// The C++ `Error::Type` enum distinguished `None`, `Errno`, `Socket`,
/// `User`, `Win32`, and `HResult`. In idiomatic Rust we collapse the
/// platform-specific cases into the more general [`Io`] / [`User`]
/// variants and add the higher-level categories (`InvalidParam`,
/// `OutOfMemory`, `Recoverable`, `None`) that the rest of PCSX2's
/// exception/macro machinery already reasons about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorKind {
    /// No error has been recorded.
    None,
    /// An I/O / OS-level error (errno, Win32, HRESULT, socket, ...).
    Io,
    /// Caller supplied an invalid parameter.
    InvalidParam,
    /// A memory allocation failed.
    OutOfMemory,
    /// A recoverable runtime error.
    Recoverable,
    /// A user-supplied / application-defined error.
    User,
}

impl Default for ErrorKind {
    fn default() -> Self {
        ErrorKind::None
    }
}

/// The standard PCSX2 error type.
///
/// `Pcsx2Error` is a thin newtype around a `String` carrying a
/// human-readable description. It deliberately avoids the original C++
/// split between a `Type` discriminator and a description string: the
/// [`ErrorKind`] enum on the sidecar [`Error`] struct (or via the
/// [`Pcsx2Error::kind`] helper) covers categorisation when needed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pcsx2Error(pub String);

impl Pcsx2Error {
    /// Create a new `Pcsx2Error` from any value that can be formatted.
    pub fn new<S: fmt::Display>(msg: S) -> Self {
        Pcsx2Error(msg.to_string())
    }

    /// Borrow the underlying description as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Categorise this error. Plain `Pcsx2Error` values default to
    /// [`ErrorKind::User`]; the [`Error`] enum can carry a more specific
    /// kind when richer context is required.
    pub fn kind(&self) -> ErrorKind {
        ErrorKind::User
    }
}

impl fmt::Display for Pcsx2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl StdError for Pcsx2Error {}

impl From<&str> for Pcsx2Error {
    fn from(s: &str) -> Self {
        Pcsx2Error(s.to_owned())
    }
}

impl From<String> for Pcsx2Error {
    fn from(s: String) -> Self {
        Pcsx2Error(s)
    }
}

impl From<io::Error> for Pcsx2Error {
    fn from(e: io::Error) -> Self {
        Pcsx2Error(e.to_string())
    }
}

impl From<fmt::Error> for Pcsx2Error {
    fn from(e: fmt::Error) -> Self {
        Pcsx2Error(format!("formatting error: {}", e))
    }
}

/// A richer error description that pairs a category with a message.
///
/// This is the closest Rust analogue to the C++ `Error` class: a
/// `Type`-discriminated value that also carries a formatted description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    description: String,
}

impl Error {
    /// Construct a new `Error` of the given [`ErrorKind`] with the supplied
    /// message.
    pub fn new(kind: ErrorKind, description: impl Into<String>) -> Self {
        Error {
            kind,
            description: description.into(),
        }
    }

    /// Construct a no-op / empty error.
    ///
    /// Idiomatic translation of `Error::CreateNone()`.
    pub fn none() -> Self {
        Error {
            kind: ErrorKind::None,
            description: String::new(),
        }
    }

    /// Construct an I/O error from an [`io::Error`].
    pub fn from_io(err: io::Error) -> Self {
        Error {
            kind: ErrorKind::Io,
            description: err.to_string(),
        }
    }

    /// Construct an errno-styled I/O error from an OS error code.
    ///
    /// Idiomatic translation of `Error::CreateErrno(int)`. Mirrors the
    /// `Error::SetErrno(int)` format: `"errno <code>: <message>"`.
    pub fn from_errno(err: i32) -> Self {
        let mut e = Error::none();
        e.set_errno(err);
        e
    }

    /// Construct a socket-styled I/O error from an OS error code.
    ///
    /// Idiomatic translation of `Error::CreateSocket(int)`. Collapses
    /// to the errno representation on every platform.
    pub fn from_socket(err: i32) -> Self {
        let mut e = Error::none();
        e.set_socket(err);
        e
    }

    /// Construct a user-defined error from a moved-in [`String`] description.
    ///
    /// Idiomatic translation of `Error::CreateString(std::string)`.
    pub fn from_string(description: impl Into<String>) -> Self {
        Error {
            kind: ErrorKind::User,
            description: description.into(),
        }
    }

    /// Construct an out-of-memory error.
    pub fn out_of_memory() -> Self {
        Error {
            kind: ErrorKind::OutOfMemory,
            description: "out of memory".to_string(),
        }
    }

    /// Construct an invalid-parameter error.
    pub fn invalid_param(msg: impl Into<String>) -> Self {
        Error {
            kind: ErrorKind::InvalidParam,
            description: msg.into(),
        }
    }

    /// Construct a recoverable error.
    pub fn recoverable(msg: impl Into<String>) -> Self {
        Error {
            kind: ErrorKind::Recoverable,
            description: msg.into(),
        }
    }

    /// Returns the [`ErrorKind`] of this error.
    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    /// Returns `true` if this error carries any information (i.e. the kind
    /// is not [`ErrorKind::None`]).
    pub fn is_valid(&self) -> bool {
        self.kind != ErrorKind::None
    }

    /// Borrow the underlying description.
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Reset this error to the empty / `None` state.
    pub fn clear(&mut self) {
        self.kind = ErrorKind::None;
        self.description.clear();
    }

    /// Set the error description from a system `errno` value (no prefix).
    ///
    /// Idiomatic translation of `Error::SetErrno(int)` from `common/Error.cpp`.
    /// The description format mirrors the C++ output: `"errno <code>: <message>"`.
    pub fn set_errno(&mut self, err: i32) {
        self.set_errno_with_prefix("", err);
    }

    /// Set the error description from a system `errno` value with a prefix.
    ///
    /// Idiomatic translation of `Error::SetErrno(std::string_view, int)`.
    /// The full description has the shape `"<prefix>errno <code>: <message>"`,
    /// or `"<prefix>errno <code>: <Could not get error message>"` if the
    /// platform failed to resolve the message.
    pub fn set_errno_with_prefix(&mut self, prefix: &str, err: i32) {
        self.kind = ErrorKind::Io;
        let msg = std::io::Error::from_raw_os_error(err);
        let body = msg.to_string();
        if body.is_empty() {
            self.description = format!("{}errno {}: <Could not get error message>", prefix, err);
        } else {
            self.description = format!("{}errno {}: {}", prefix, err, body);
        }
    }

    /// Set the error description from a borrowed string view.
    ///
    /// Idiomatic translation of `Error::SetStringView(std::string_view)`.
    pub fn set_string_view(&mut self, description: &str) {
        self.kind = ErrorKind::User;
        self.description.clear();
        self.description.push_str(description);
    }

    /// Set the error description from a socket/system error code.
    ///
    /// Idiomatic translation of `Error::SetSocket(int)`. On Windows the
    /// socket error is the same as a Win32 error; elsewhere it is the same
    /// as an errno. The Rust translation collapses both into the generic
    /// [`ErrorKind::Io`] variant.
    pub fn set_socket(&mut self, err: i32) {
        self.set_socket_with_prefix("", err);
    }

    /// Set the error description from a socket/system error code with a prefix.
    ///
    /// Idiomatic translation of `Error::SetSocket(std::string_view, int)`.
    pub fn set_socket_with_prefix(&mut self, prefix: &str, err: i32) {
        self.set_errno_with_prefix(prefix, err);
        // Socket errors are still an I/O error in Rust; the C++ version
        // distinguishes Socket from Errno via Type::Socket, but we collapse
        // them into ErrorKind::Io per the documented translation policy.
        let _ = prefix;
        let _ = err;
    }

    /// Prepend a prefix to the description.
    pub fn add_prefix(&mut self, prefix: &str) {
        self.description.insert_str(0, prefix);
    }

    /// Append a suffix to the description.
    pub fn add_suffix(&mut self, suffix: &str) {
        self.description.push_str(suffix);
    }
}

// ---------------------------------------------------------------------------
// Static helpers (the C++ `Error::Foo(Error* errptr, ...)` overloads).
//
// In idiomatic Rust these do not need to be methods of `Error`; they are
// free functions that mirror the C++ pattern of "if `errptr` is non-null,
// set the error on it; otherwise no-op". Exposing them as free functions
// keeps the `Error` API minimal while preserving the call-site shape
// (`Error::clear_ptr(&mut err)` vs `Error::Clear(&mut err)`).
// ---------------------------------------------------------------------------

/// Reset `err` to the empty state if it is non-null.
///
/// Idiomatic translation of `Error::Clear(Error*)`.
pub fn clear_ptr(err: Option<&mut Error>) {
    if let Some(e) = err {
        e.clear();
    }
}

/// Apply `set_errno(err)` on `errptr` if it is non-null.
///
/// Idiomatic translation of `Error::SetErrno(Error*, int)`.
pub fn set_errno_ptr(errptr: Option<&mut Error>, err: i32) {
    if let Some(e) = errptr {
        e.set_errno(err);
    }
}

/// Apply `set_errno_with_prefix(prefix, err)` on `errptr` if it is non-null.
///
/// Idiomatic translation of `Error::SetErrno(Error*, std::string_view, int)`.
pub fn set_errno_with_prefix_ptr(errptr: Option<&mut Error>, prefix: &str, err: i32) {
    if let Some(e) = errptr {
        e.set_errno_with_prefix(prefix, err);
    }
}

/// Apply `set_string_view(desc)` on `errptr` if it is non-null.
///
/// Idiomatic translation of `Error::SetStringView(Error*, std::string_view)`.
pub fn set_string_view_ptr(errptr: Option<&mut Error>, description: &str) {
    if let Some(e) = errptr {
        e.set_string_view(description);
    }
}

/// Apply `set_socket(err)` on `errptr` if it is non-null.
///
/// Idiomatic translation of `Error::SetSocket(Error*, int)`.
pub fn set_socket_ptr(errptr: Option<&mut Error>, err: i32) {
    if let Some(e) = errptr {
        e.set_socket(err);
    }
}

/// Apply `set_socket_with_prefix(prefix, err)` on `errptr` if it is non-null.
///
/// Idiomatic translation of `Error::SetSocket(Error*, std::string_view, int)`.
pub fn set_socket_with_prefix_ptr(errptr: Option<&mut Error>, prefix: &str, err: i32) {
    if let Some(e) = errptr {
        e.set_socket_with_prefix(prefix, err);
    }
}

/// Apply `set_string(desc)` on `errptr` if it is non-null.
///
/// Idiomatic translation of `Error::SetString(Error*, std::string)`. The
/// `String` is moved into the destination, so we consume the argument.
pub fn set_string_ptr(errptr: Option<&mut Error>, description: String) {
    if let Some(e) = errptr {
        e.kind = ErrorKind::User;
        e.description = description;
    }
}

/// Apply `add_prefix(prefix)` on `errptr` if it is non-null.
///
/// Idiomatic translation of `Error::AddPrefix(Error*, std::string_view)`.
pub fn add_prefix_ptr(errptr: Option<&mut Error>, prefix: &str) {
    if let Some(e) = errptr {
        e.add_prefix(prefix);
    }
}

/// Apply `add_suffix(suffix)` on `errptr` if it is non-null.
///
/// Idiomatic translation of `Error::AddSuffix(Error*, std::string_view)`.
pub fn add_suffix_ptr(errptr: Option<&mut Error>, suffix: &str) {
    if let Some(e) = errptr {
        e.add_suffix(suffix);
    }
}

impl Default for Error {
    fn default() -> Self {
        Error::none()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description)
    }
}

impl StdError for Error {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        None
    }
}

impl From<Pcsx2Error> for Error {
    fn from(e: Pcsx2Error) -> Self {
        Error {
            kind: e.kind(),
            description: e.0,
        }
    }
}

impl From<Error> for Pcsx2Error {
    fn from(e: Error) -> Self {
        Pcsx2Error(e.description)
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::from_io(e)
    }
}

/// An unrecoverable condition with a captured backtrace.
///
/// `Pcsx2Exception` is the Rust translation of PCSX2's exception hierarchy
/// (`BaseException`, `RuntimeError`, `RecoverableError`, ...). Unlike
/// [`Pcsx2Error`] / [`Error`], constructing an exception is meant to be a
/// terminating event — the captured backtrace makes the failure site easy
/// to diagnose in logs and crash dumps.
#[derive(Debug)]
pub struct Pcsx2Exception {
    message: String,
    backtrace: Backtrace,
}

impl Pcsx2Exception {
    /// Capture a new exception with a backtrace at the current call site.
    pub fn new(message: impl Into<String>) -> Self {
        Pcsx2Exception {
            message: message.into(),
            backtrace: Backtrace::capture(),
        }
    }

    /// Borrow the exception's message.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Borrow the captured backtrace.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }

    /// `true` if the backtrace was actually captured (i.e. `RUST_BACKTRACE`
    /// was set and the platform supports it).
    pub fn has_backtrace(&self) -> bool {
        self.backtrace.status() == BacktraceStatus::Captured
    }
}

impl fmt::Display for Pcsx2Exception {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl StdError for Pcsx2Exception {}

/// A runtime exception — the default "this should not have happened" signal.
#[derive(Debug)]
pub struct RuntimeError(Pcsx2Exception);

impl RuntimeError {
    pub fn new(message: impl Into<String>) -> Self {
        RuntimeError(Pcsx2Exception::new(message))
    }

    pub fn message(&self) -> &str {
        self.0.message()
    }

    pub fn backtrace(&self) -> &Backtrace {
        self.0.backtrace()
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl StdError for RuntimeError {}

/// An exception that signals a recoverable failure.
///
/// The original C++ distinguishes `RecoverableError` from `RuntimeError`
/// primarily for assertions / log tagging; on the Rust side we keep the
/// types separate so callers can pattern-match on intent.
#[derive(Debug)]
pub struct RecoverableError(Pcsx2Exception);

impl RecoverableError {
    pub fn new(message: impl Into<String>) -> Self {
        RecoverableError(Pcsx2Exception::new(message))
    }

    pub fn message(&self) -> &str {
        self.0.message()
    }

    pub fn backtrace(&self) -> &Backtrace {
        self.0.backtrace()
    }
}

impl fmt::Display for RecoverableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl StdError for RecoverableError {}

/// Panics with a formatted message.
///
/// Idiomatic translation of the C++ `pxAssume` / `pxAssert` family of
/// macros: all of them are "this should never happen" signals that we
/// express by raising a [`RuntimeError`] / [`Pcsx2Exception`] (in a
/// non-`#[no_std]` `catch_unwind` world) or, when `abort-on-panic` is
/// desired, by panicking. We use `panic!` here so the standard tooling
/// (panic hook, backtrace, minidump) does the heavy lifting.
#[macro_export]
macro_rules! pxAssume {
    ($cond:expr $(,)?) => {
        if !$cond {
            $crate::common::Error::px_assertion_failure(stringify!($cond));
        }
    };
    ($cond:expr, $($arg:tt)+) => {
        if !$cond {
            $crate::common::Error::px_assertion_failure(format!($($arg)+));
        }
    };
}

/// `pxAssert` is currently a thin alias for `pxAssume` — the C++ codebase
/// uses both names for the same intent (the historical distinction was a
/// debug-only toggle).
#[macro_export]
macro_rules! pxAssert {
    ($($tt:tt)+) => {
        $crate::pxAssume!($($tt)+)
    };
}

impl Pcsx2Error {
    /// Internal helper used by the `pxAssume` / `pxAssert` macros.
    #[doc(hidden)]
    #[cold]
    #[inline(never)]
    pub fn px_assertion_failure(msg: impl Into<String>) -> ! {
        panic!("Assumption failed: {}", msg.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcsx2_error_displays_message() {
        let e = Pcsx2Error("boom".to_string());
        assert_eq!(e.to_string(), "boom");
        assert_eq!(e.as_str(), "boom");
        assert!(e.source().is_none());
    }

    #[test]
    fn from_io_error_round_trip() {
        let io_err = io::Error::new(io::ErrorKind::NotFound, "missing");
        let err: Pcsx2Error = io_err.into();
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn error_kind_lifecycle() {
        let mut e = Error::none();
        assert!(!e.is_valid());
        assert_eq!(e.kind(), ErrorKind::None);

        e = Error::out_of_memory();
        assert!(e.is_valid());
        assert_eq!(e.kind(), ErrorKind::OutOfMemory);

        e.add_prefix("oops: ");
        assert_eq!(e.description(), "oops: out of memory");
        e.add_suffix(" (try again)");
        assert_eq!(e.description(), "oops: out of memory (try again)");

        e.clear();
        assert!(!e.is_valid());
    }

    #[test]
    fn exception_captures_backtrace() {
        let exc = Pcsx2Exception::new("nope");
        assert_eq!(exc.message(), "nope");
    }

    #[test]
    fn errno_helpers_format_message() {
        let e = Error::from_errno(2); // ENOENT on Unix; on Windows the message text differs but the kind must be Io.
        assert_eq!(e.kind(), ErrorKind::Io);
        assert!(e.description().starts_with("errno 2:"));

        let mut e = Error::none();
        e.set_errno_with_prefix("open: ", 2);
        assert_eq!(e.kind(), ErrorKind::Io);
        assert!(e.description().starts_with("open: errno 2:"));
    }

    #[test]
    fn string_and_socket_helpers() {
        let mut e = Error::none();
        e.set_string_view("hello world");
        assert_eq!(e.kind(), ErrorKind::User);
        assert_eq!(e.description(), "hello world");

        let e = Error::from_socket(111); // ECONNREFUSED on Unix
        assert_eq!(e.kind(), ErrorKind::Io);
        assert!(e.description().starts_with("errno 111:"));

        let e = Error::from_string("oops".to_string());
        assert_eq!(e.kind(), ErrorKind::User);
        assert_eq!(e.description(), "oops");
    }

    #[test]
    fn static_ptr_helpers_noop_on_none() {
        clear_ptr(None);
        set_errno_ptr(None, 1);
        set_errno_with_prefix_ptr(None, "p: ", 1);
        set_string_view_ptr(None, "x");
        set_string_ptr(None, "x".to_string());
        set_socket_ptr(None, 1);
        set_socket_with_prefix_ptr(None, "p: ", 1);
        add_prefix_ptr(None, "p: ");
        add_suffix_ptr(None, " :s");
        // nothing to assert other than the absence of panic.
    }

    #[test]
    fn static_ptr_helpers_apply_when_some() {
        let mut e = Error::none();
        set_errno_ptr(Some(&mut e), 2);
        assert_eq!(e.kind(), ErrorKind::Io);

        let mut e = Error::none();
        set_errno_with_prefix_ptr(Some(&mut e), "pre: ", 2);
        assert!(e.description().starts_with("pre: errno 2:"));

        let mut e = Error::none();
        set_string_view_ptr(Some(&mut e), "hi");
        assert_eq!(e.description(), "hi");
        assert_eq!(e.kind(), ErrorKind::User);

        let mut e = Error::none();
        set_string_ptr(Some(&mut e), "owned".to_string());
        assert_eq!(e.description(), "owned");
        assert_eq!(e.kind(), ErrorKind::User);

        let mut e = Error::none();
        set_socket_ptr(Some(&mut e), 111);
        assert_eq!(e.kind(), ErrorKind::Io);

        let mut e = Error::none();
        set_socket_with_prefix_ptr(Some(&mut e), "sock: ", 111);
        assert!(e.description().starts_with("sock: errno 111:"));

        let mut e = Error::from_string("body".to_string());
        add_prefix_ptr(Some(&mut e), "[pre] ");
        add_suffix_ptr(Some(&mut e), " [post]");
        assert_eq!(e.description(), "[pre] body [post]");

        let mut e = Error::from_string("body".to_string());
        clear_ptr(Some(&mut e));
        assert!(!e.is_valid());
    }

    #[test]
    fn equality_matches_type_and_description() {
        let a = Error::from_string("same".to_string());
        let b = Error::from_string("same".to_string());
        let c = Error::from_string("different".to_string());
        assert_eq!(a, b);
        assert_ne!(a, c);

        let d = Error::from_errno(2);
        let e2 = Error::from_errno(2);
        assert_eq!(d, e2);
    }
}
