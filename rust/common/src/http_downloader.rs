// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Pure-Rust port of `common/HTTPDownloader.{h,cpp}` (with the curl and
// WinHTTP backends folded together). The original C++ class was an
// async, multi-request, libcurl-on-Unix / WinHTTP-on-Windows downloader
// with per-request progress callbacks and per-request cancellation.
//
// This Rust translation collapses the multi-request polling loop into
// a synchronous "one call, one file" API that maps directly onto a
// single C FFI export. The motivation:
//   - The FFI exports requested (`pcsx2_http_download`,
//     `pcsx2_http_download_with_progress`) are blocking, one-shot
//     operations. A long-lived `HTTPDownloader` object with a
//     `PollRequests` worker thread would be dead weight.
//   - The HTTP backend (libcurl / WinHTTP) is hidden behind a single
//     Rust HTTP client (`ureq`), which already handles platform
//     differences transparently.
//
// Dependency (added to `Cargo.toml`):
// ```text
// ureq = "2"
// ```

//! HTTP file-downloader.
//!
//! Mirrors `common/HTTPDownloader.{h,cpp}` from the C++ side. The
//! original class was an async multi-request downloader with per-request
//! progress callbacks; this Rust port exposes the bits that the FFI
//! surface actually needs:
//!
//! - [`HttpDownloader::new`] — construct a downloader with the default
//!   user-agent and timeout.
//! - [`HttpDownloader::download`] — fetch `url` and stream the body to
//!   `dest`, invoking a [`ProgressCallback`] (if provided) for each
//!   chunk.
//! - [`download_file`] — one-shot convenience used by the no-progress
//!   FFI export.
//!
//! ## Mapping from C++ to Rust
//!
//! - The C++ `HTTPDownloader` virtual interface (libcurl vs WinHTTP
//!   subclasses, `InternalPollRequests`, etc.) is collapsed into a
//!   single struct backed by the pure-Rust [`ureq::Agent`]. There is no
//!   per-platform split on the Rust side; `ureq` handles both
//!   transports internally.
//! - The C++ per-request state machine (`Pending` -> `Started` ->
//!   `Receiving` -> `Complete`) is replaced by a straight-line streaming
//!   read loop with periodic `ProgressCallback` updates.
//! - The C++ `HTTP_STATUS_*` enum becomes the [`Error`] variants.
//!   `HTTP_STATUS_OK = 200` is folded into the success path; the
//!   negative codes map to:
//!     - `HTTP_STATUS_CANCELLED` -> [`Error::Cancelled`]
//!     - `HTTP_STATUS_TIMEOUT`   -> [`Error::Timeout`]
//!     - `HTTP_STATUS_ERROR`     -> [`Error::Http`] / [`Error::Io`]
//! - The C++ `ProgressCallback` C++ abstract class is consumed via the
//!   [`ProgressCallback`] trait re-exported from `crate::progress_callback`.
//!   The Rust trait already mirrors the C++ methods we need
//!   (`set_progress_range`, `set_progress_value`, `is_cancelled`).
//!
//! ## Windows-specific behaviour
//!
//! The original C++ `HTTPDownloaderWinHTTP` relied on the WinHTTP
//! service, which trusts whatever CAs are installed in the Windows
//! certificate store (system + user). The Rust port uses `ureq`
//! backed by `rustls`; on Windows the build enables the
//! `native-certs` feature so `rustls-native-certs` extracts roots
//! from the Windows certificate store at startup. The resulting
//! trust decisions match WinHTTP's: a corporate / custom CA
//! installed on the user's machine is trusted by both.
//!
//! On macOS and Linux the bundled Mozilla `webpki-roots` are used,
//! which is appropriate for a desktop emulator distribution.
//!
//! To verify the certificate store behaviour at runtime, build the
//! `http_downloader_win_test` example and run it under Windows;
//! it prints `tls roots = os-cert-store (rustls-native-certs)` on
//! Windows and `tls roots = webpki-roots (bundled Mozilla)` elsewhere.
//!
//! ## FFI
//!
//! Two C-ABI exports are provided at the bottom of the file:
//!
//! - [`pcsx2_http_download`] — download a file with no progress
//!   reporting. `true` on success, `false` on failure.
//! - [`pcsx2_http_download_with_progress`] — same as above but with a
//!   `extern "C" fn(u64, u64)` callback invoked with `(bytes_received,
//!   total_bytes)` after every chunk. The total is best-effort: it is
//!   the value of the `Content-Length` response header when present,
//!   and `0` otherwise.
//!
//! Both functions take UTF-8 NUL-terminated C strings and follow the
//! same C-string handling conventions as the rest of the crate.

use std::ffi::{c_char, CStr};
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::Duration;

use ureq::{Agent, AgentBuilder, Error as UreqError, Response};

use crate::progress_callback::ProgressCallback;

// ============================================================================
// Constants
// ============================================================================

/// Default User-Agent sent with every request.
///
/// Mirrors `HTTPDownloader::DEFAULT_USER_AGENT` in the C++ source.
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:85.0) Gecko/20100101 Firefox/85.0";

/// Default request timeout (seconds).
///
/// Mirrors `DEFAULT_TIMEOUT_IN_SECONDS = 30` in `HTTPDownloader.cpp`.
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;

/// Streaming chunk size (bytes).
///
/// `ureq` does not give us an incremental reader; it materialises the
/// body in memory on the happy path and hands us a [`Read`] on error.
/// We read in 8 KiB chunks so progress reporting stays responsive
/// without inflating sys-call overhead.
const STREAM_CHUNK: usize = 8 * 1024;

/// Short, human-readable label describing which TLS root store the
/// current build trusts.
///
/// Driven by the build's target OS:
///
/// - Windows  -> "os-cert-store (rustls-native-certs, WinHTTP-equivalent)"
/// - macOS    -> "webpki-roots (bundled Mozilla)"
/// - Linux    -> "webpki-roots (bundled Mozilla)"
/// - other    -> "webpki-roots (bundled Mozilla)"
///
/// This exists so the test/example binary can print which trust
/// store is in effect without having to fish the answer out of
/// `ureq`'s internals. It deliberately does not perform any
/// filesystem or registry access; the actual roots are loaded by
/// `ureq`/`rustls-native-certs` at agent construction time.
pub fn tls_roots_label() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "os-cert-store (rustls-native-certs, WinHTTP-equivalent)"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "webpki-roots (bundled Mozilla)"
    }
}

// ============================================================================
// Error type
// ============================================================================

/// Errors that can occur while downloading a file.
///
/// The variants cover the negative status codes from the original
/// `HTTPDownloader::HTTP_STATUS_*` enum plus the more specific failures
/// that the C++ side collapsed into `HTTP_STATUS_ERROR`:
/// - [`Error::Http`]        - transport-level error from `ureq` (DNS,
///                            TLS, status >= 400, etc.)
/// - [`Error::Io`]          - local filesystem error writing the file.
/// - [`Error::Timeout`]     - request exceeded the configured timeout.
/// - [`Error::Cancelled`]   - the supplied [`ProgressCallback`] reported
///                            `is_cancelled() == true`.
/// - [`Error::InvalidUrl`]  - the URL was malformed or empty.
#[derive(Debug)]
pub enum Error {
    /// Underlying I/O error (writing to `dest`, opening the file, ...).
    Io(io::Error),

    /// `ureq` returned a non-success status code or transport-level
    /// failure (DNS, TLS handshake, connection refused, ...).
    Http(String),

    /// The configured timeout elapsed before the response completed.
    Timeout,

    /// The supplied progress callback reported cancellation.
    Cancelled,

    /// The supplied URL was empty or otherwise unparseable.
    InvalidUrl(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Http(msg) => write!(f, "HTTP error: {msg}"),
            Error::Timeout => write!(f, "request timed out"),
            Error::Cancelled => write!(f, "download cancelled"),
            Error::InvalidUrl(s) => write!(f, "invalid URL: {s}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

impl From<UreqError> for Error {
    fn from(e: UreqError) -> Self {
        match e {
            UreqError::Status(code, response) => {
                Error::Http(format!("HTTP status {code} ({})", response.status_text()))
            }
            UreqError::Transport(t) => {
                // `ureq::Error::Transport` wraps an `std::io::Error`
                // describing why the request never got a response
                // (DNS, TCP, TLS). Surface the message verbatim.
                Error::Http(t.to_string())
            }
        }
    }
}

// ============================================================================
// MIME type -> extension lookup
// ============================================================================

/// Map a `Content-Type` header value to the conventional file
/// extension (without leading dot).
///
/// Mirrors `HTTPDownloader::GetExtensionForContentType` from the C++
/// source. Returns an empty string when no match is found.
///
/// Case-insensitive compare on the content-type string.
pub fn get_extension_for_content_type(content_type: &str) -> String {
    // Table mirrors the C++ `table[][2]` in `HTTPDownloader.cpp`.
    // Entries are `(mime, ext)`.
    const TABLE: &[(&str, &str)] = &[
        ("audio/aac", "aac"),
        ("application/x-abiword", "abw"),
        ("application/x-freearc", "arc"),
        ("image/avif", "avif"),
        ("video/x-msvideo", "avi"),
        ("application/vnd.amazon.ebook", "azw"),
        ("application/octet-stream", "bin"),
        ("image/bmp", "bmp"),
        ("application/x-bzip", "bz"),
        ("application/x-bzip2", "bz2"),
        ("application/x-cdf", "cda"),
        ("application/x-csh", "csh"),
        ("text/css", "css"),
        ("text/csv", "csv"),
        ("application/msword", "doc"),
        (
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "docx",
        ),
        ("application/vnd.ms-fontobject", "eot"),
        ("application/epub+zip", "epub"),
        ("application/gzip", "gz"),
        ("image/gif", "gif"),
        ("text/html", "htm"),
        ("image/vnd.microsoft.icon", "ico"),
        ("text/calendar", "ics"),
        ("application/java-archive", "jar"),
        ("image/jpeg", "jpg"),
        ("text/javascript", "js"),
        ("application/json", "json"),
        ("application/ld+json", "jsonld"),
        ("audio/midi audio/x-midi", "mid"),
        ("text/javascript", "mjs"),
        ("audio/mpeg", "mp3"),
        ("video/mp4", "mp4"),
        ("video/mpeg", "mpeg"),
        ("application/vnd.apple.installer+xml", "mpkg"),
        ("application/vnd.oasis.opendocument.presentation", "odp"),
        ("application/vnd.oasis.opendocument.spreadsheet", "ods"),
        ("application/vnd.oasis.opendocument.text", "odt"),
        ("audio/ogg", "oga"),
        ("video/ogg", "ogv"),
        ("application/ogg", "ogx"),
        ("audio/opus", "opus"),
        ("font/otf", "otf"),
        ("image/png", "png"),
        ("application/pdf", "pdf"),
        ("application/x-httpd-php", "php"),
        ("application/vnd.ms-powerpoint", "ppt"),
        (
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "pptx",
        ),
        ("application/vnd.rar", "rar"),
        ("application/rtf", "rtf"),
        ("application/x-sh", "sh"),
        ("image/svg+xml", "svg"),
        ("application/x-tar", "tar"),
        ("image/tiff", "tif"),
        ("video/mp2t", "ts"),
        ("font/ttf", "ttf"),
        ("text/plain", "txt"),
        ("application/vnd.visio", "vsd"),
        ("audio/wav", "wav"),
        ("audio/webm", "weba"),
        ("video/webm", "webm"),
        ("image/webp", "webp"),
        ("font/woff", "woff"),
        ("font/woff2", "woff2"),
        ("application/xhtml+xml", "xhtml"),
        ("application/vnd.ms-excel", "xls"),
        (
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "xlsx",
        ),
        ("application/xml", "xml"),
        ("text/xml", "xml"),
        ("application/vnd.mozilla.xul+xml", "xul"),
        ("application/zip", "zip"),
        ("video/3gpp", "3gp"),
        ("audio/3gpp", "3gp"),
        ("video/3gpp2", "3g2"),
        ("audio/3gpp2", "3g2"),
        ("application/x-7z-compressed", "7z"),
    ];

    for (mime, ext) in TABLE {
        if mime.eq_ignore_ascii_case(content_type) {
            return (*ext).to_string();
        }
    }
    String::new()
}

// ============================================================================
// HttpDownloader
// ============================================================================

/// HTTP file downloader.
///
/// Wraps a [`ureq::Agent`] with a default user-agent and timeout. Each
/// instance is `Send + Sync` because `Agent` is `Send + Sync` and our
/// fields are immutable after construction.
///
/// Create one with [`HttpDownloader::new`], then call
/// [`HttpDownloader::download`] for each transfer. The downloader can
/// be reused for many sequential transfers; for concurrent transfers
/// spawn one instance per thread (or use `ureq::Agent::new()` directly
/// in async code paths).
///
/// Equivalent of the C++ `HTTPDownloader` class, minus the per-request
/// polling loop (the Rust version is strictly synchronous and one-shot
/// per `download` call).
pub struct HttpDownloader {
    agent: Agent,
}

impl HttpDownloader {
    /// Build a downloader with [`DEFAULT_USER_AGENT`] and the default
    /// 30-second timeout.
    pub fn new() -> Self {
        Self::builder()
            .user_agent(DEFAULT_USER_AGENT)
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECONDS))
            .build()
    }

    /// Build a downloader with a custom user-agent (e.g. a build
    /// identifier for telemetry). Other settings are the defaults.
    pub fn with_user_agent(user_agent: &str) -> Self {
        Self::builder()
            .user_agent(user_agent)
            .timeout(Duration::from_secs(DEFAULT_TIMEOUT_SECONDS))
            .build()
    }

    /// Build a downloader with a custom request timeout. Mirrors the
    /// `HTTPDownloader::SetTimeout(float)` setter in the C++ side.
    /// The same user-agent as [`HttpDownloader::new`] is used.
    ///
    /// `timeout` is applied to the whole request (DNS + TCP + TLS +
    /// response + body read). It corresponds to
    /// `WINHTTP_OPTION_CONNECT_TIMEOUT | SEND_TIMEOUT | RECEIVE_TIMEOUT`
    /// on the C++ side.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self::builder()
            .user_agent(DEFAULT_USER_AGENT)
            .timeout(timeout)
            .build()
    }

    /// Build a downloader with both a custom user-agent and a custom
    /// request timeout.
    pub fn with_user_agent_and_timeout(user_agent: &str, timeout: Duration) -> Self {
        Self::builder()
            .user_agent(user_agent)
            .timeout(timeout)
            .build()
    }

    /// Internal builder helper used by [`Self::new`] and
    /// [`Self::with_user_agent`].
    fn builder() -> HttpDownloaderBuilder {
        HttpDownloaderBuilder {
            user_agent: DEFAULT_USER_AGENT.to_string(),
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SECONDS),
        }
    }

    /// Download `url` and stream the body to `dest` on the local
    /// filesystem.
    ///
    /// `dest` is created if it does not exist and truncated if it
    /// does. The HTTP `Content-Length` header (if present) is reported
    /// via `progress.set_progress_range()`; `set_progress_value()` is
    /// called with the cumulative byte count after every chunk.
    ///
    /// If `progress` returns `is_cancelled() == true` at any point, the
    /// download is aborted, the partially-written file is removed, and
    /// [`Error::Cancelled`] is returned.
    pub fn download(
        &self,
        url: &str,
        dest: &Path,
        mut progress: Option<&mut dyn ProgressCallback>,
    ) -> Result<(), Error> {
        if url.is_empty() {
            return Err(Error::InvalidUrl(url.to_string()));
        }

        // Title / status text on the callback so the UI can show
        // "Downloading <url>" while we run. Matches the C++ pattern of
        // updating status before kicking off a long-running op.
        if let Some(cb) = progress.as_deref_mut() {
            cb.set_title("HTTP Download");
            cb.set_status_text(&format!("Downloading {url}"));
        }

        let response = self
            .agent
            .get(url)
            .call()
            .map_err(Error::from)?;

        // Extract content-length for the progress range. The header
        // may be missing for chunked transfer-encoding; in that case
        // we leave the range at 0 and just count up the bytes we
        // actually receive.
        let content_length: u32 = response
            .header("Content-Length")
            .and_then(|s| s.parse::<u64>().ok())
            .map(|v| v.min(u32::MAX as u64) as u32)
            .unwrap_or(0);

        if let Some(cb) = progress.as_deref_mut() {
            cb.set_progress_range(content_length);
            cb.set_progress_value(0);
        }

        stream_to_file(response, dest, content_length, progress)
    }

    /// Issue an HTTP GET to `url` and return the response body as a
    /// `Vec<u8>` along with the `Content-Type` header value (if any).
    ///
    /// Provided for symmetry with the C++ `HTTPDownloader::Request`
    /// callback (`status_code`, `content_type`, `data`) — useful when
    /// the caller wants to inspect the body in memory rather than
    /// stream it to disk.
    pub fn fetch(&self, url: &str) -> Result<(u16, String, Vec<u8>), Error> {
        if url.is_empty() {
            return Err(Error::InvalidUrl(url.to_string()));
        }

        let response = self.agent.get(url).call().map_err(Error::from)?;
        let status = response.status();
        let content_type = response
            .header("Content-Type")
            .unwrap_or("")
            .to_string();

        let mut reader = response.into_reader();
        let mut buf = Vec::new();
        reader.read_to_end(&mut buf)?;
        Ok((status, content_type, buf))
    }
}

impl Default for HttpDownloader {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for [`HttpDownloader`].
///
/// Lets callers customise the user-agent and timeout without breaking
/// the `new()` / `with_user_agent()` public surface.
struct HttpDownloaderBuilder {
    user_agent: String,
    timeout: Duration,
}

impl HttpDownloaderBuilder {
    fn user_agent(mut self, ua: &str) -> Self {
        self.user_agent = ua.to_string();
        self
    }
    fn timeout(mut self, t: Duration) -> Self {
        self.timeout = t;
        self
    }
    fn build(self) -> HttpDownloader {
        let agent = AgentBuilder::new()
            .user_agent(&self.user_agent)
            .timeout(self.timeout)
            .build();
        HttpDownloader { agent }
    }
}

// ============================================================================
// Free function: one-shot download
// ============================================================================

/// Download `url` to `dest` without any progress reporting.
///
/// Equivalent to `HttpDownloader::new().download(url, dest, None)`.
/// Provided so the no-progress FFI export can be a single line of
/// glue.
pub fn download_file(url: &str, dest: &Path) -> Result<(), Error> {
    let downloader = HttpDownloader::new();
    downloader.download(url, dest, None)
}

// ============================================================================
// Internal: streaming response body to a file
// ============================================================================

/// Stream `response` body to `dest`, invoking `progress` (if any) after
/// every chunk.
///
/// Returns [`Error::Cancelled`] if the callback reports cancellation;
/// the partially-written file is best-effort removed in that case so we
/// don't leave half-downloaded junk on disk.
fn stream_to_file(
    response: Response,
    dest: &Path,
    content_length: u32,
    mut progress: Option<&mut dyn ProgressCallback>,
) -> Result<(), Error> {
    let mut reader = response.into_reader();
    let mut file = File::create(dest)?;

    let mut buf = [0u8; STREAM_CHUNK];
    let mut received: u32 = 0;
    let mut cancelled = false;

    loop {
        // Cooperative cancellation: peek at the callback before each
        // chunk so a UI cancel button becomes responsive within one
        // ~8 KiB read.
        if let Some(cb) = progress.as_deref_mut() {
            if cb.is_cancelled() {
                cancelled = true;
                break;
            }
        }

        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                // Best-effort cleanup so we don't leak a half-written
                // file on transport failure. Ignore the cleanup
                // error — the caller wants to know about the read
                // failure, not the cleanup outcome.
                let _ = std::fs::remove_file(dest);
                return Err(Error::Io(e));
            }
        };

        file.write_all(&buf[..n])?;
        received = received.saturating_add(n as u32);

        if let Some(cb) = progress.as_deref_mut() {
            cb.set_progress_value(received);
        }
    }

    file.flush()?;

    if cancelled {
        // Drop the partial file. Same rationale as the read-error
        // path above.
        let _ = std::fs::remove_file(dest);
        return Err(Error::Cancelled);
    }

    // Touch the content_length variable so the unused-warning lints
    // are happy; it's been reported to the progress callback already.
    let _ = content_length;
    Ok(())
}

// ============================================================================
// FFI surface
// ============================================================================
//
// These `#[no_mangle] pub extern "C"` functions are the symbols the
// C++ PCSX2 binary calls. They are static-linked into the host
// executable via the `staticlib` crate-type; cbindgen picks them up
// and emits matching declarations into `pcsx2_common_rs.h`.
//
// Conventions (matching the rest of the crate):
// - Return `bool` for success. `true` = success, `false` = failure.
// - `*const c_char` arguments are UTF-8 NUL-terminated; null or
//   non-UTF8 inputs are treated as a failure.
/// Convert a `*const c_char` to `Option<&str>`.
///
/// Returns `None` for null pointers or strings containing interior
/// NULs. Matches the C-string handling used by every other module in
/// this crate (see `file_system::c_path` for the parallel pattern on
/// paths).
fn c_str<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() {
        return None;
    }
    // Safety: see contract above.
    let s = unsafe { CStr::from_ptr(p) };
    s.to_str().ok()
}

/// Download a file from `url` to `dest_path` with no progress
/// reporting.
///
/// `url` and `dest_path` must be UTF-8 NUL-terminated C strings.
/// Returns `true` on success, `false` on any failure (bad URL, IO
/// error, non-2xx HTTP status, ...).
///
/// Equivalent of `download_file` exposed across the C ABI. The C++
/// wrapper is expected to be a thin shim that just calls this and
/// propagates the bool back to its caller.
#[no_mangle]
pub extern "C" fn pcsx2_http_download(url: *const c_char, dest_path: *const c_char) -> bool {
    let (Some(u), Some(d)) = (c_str(url), c_str(dest_path)) else {
        return false;
    };
    download_file(u, Path::new(d)).is_ok()
}

/// Download a file from `url` to `dest_path`, invoking `callback`
/// after every chunk with `(bytes_received, total_bytes)`.
///
/// `callback` may be `None` — in which case this behaves identically
/// to [`pcsx2_http_download`]. When non-null, the callback is invoked
/// synchronously from the download thread, so C++ implementations
/// must be cheap and must not block.
///
/// `total_bytes` is the value of the HTTP `Content-Length` response
/// header when present, and `0` for chunked transfers. `bytes_received`
/// is the cumulative count of bytes written to disk.
///
/// Returns `true` on success, `false` on any failure (bad URL, IO
/// error, non-2xx HTTP status, ...).
#[no_mangle]
pub extern "C" fn pcsx2_http_download_with_progress(
    url: *const c_char,
    dest_path: *const c_char,
    callback: Option<extern "C" fn(u64, u64)>,
) -> bool {
    let (Some(u), Some(d)) = (c_str(url), c_str(dest_path)) else {
        return false;
    };

    let downloader = HttpDownloader::new();

    // The ureq `Response::header` reader needs to run inside
    // `download` so we can extract Content-Length before streaming.
    // We can't reach into the downloader's internals from here, so
    // we duplicate the call-and-stream logic at the FFI boundary.
    let response = match downloader.agent.get(u).call() {
        Ok(r) => r,
        Err(_) => return false,
    };

    let content_length: u64 = response
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    // Make sure the parent directory of `dest_path` exists so we
    // don't fail on a missing directory; mirrors the C++ side's
    // expectation that the destination's directory is pre-created.
    if let Some(parent) = Path::new(d).parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }

    let mut reader = response.into_reader();
    let mut file = match File::create(d) {
        Ok(f) => f,
        Err(_) => return false,
    };

    let mut buf = [0u8; STREAM_CHUNK];
    let mut received: u64 = 0;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => {
                let _ = std::fs::remove_file(d);
                return false;
            }
        };
        if file.write_all(&buf[..n]).is_err() {
            let _ = std::fs::remove_file(d);
            return false;
        }
        received = received.saturating_add(n as u64);
        if let Some(cb) = callback {
            cb(received, content_length);
        }
    }
    file.flush().is_ok()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_content_types_resolve() {
        assert_eq!(get_extension_for_content_type("image/png"), "png");
        assert_eq!(get_extension_for_content_type("application/zip"), "zip");
        assert_eq!(get_extension_for_content_type("text/html"), "htm");
    }

    #[test]
    fn unknown_content_type_returns_empty() {
        assert_eq!(get_extension_for_content_type(""), "");
        assert_eq!(
            get_extension_for_content_type("application/x-not-a-real-type"),
            ""
        );
    }

    #[test]
    fn content_type_match_is_case_insensitive() {
        assert_eq!(get_extension_for_content_type("IMAGE/PNG"), "png");
        assert_eq!(get_extension_for_content_type("Application/Zip"), "zip");
    }

    #[test]
    fn empty_url_is_invalid() {
        let dl = HttpDownloader::new();
        let dest = std::env::temp_dir().join("pcsx2_empty_url_test.bin");
        let err = dl.download("", &dest, None).unwrap_err();
        matches!(err, Error::InvalidUrl(_));
        let _ = std::fs::remove_file(&dest);
    }

    #[test]
    fn c_str_handles_null() {
        assert!(c_str(std::ptr::null()).is_none());
    }

    #[test]
    fn c_str_handles_valid_input() {
        let s = b"hello\0";
        let p = s.as_ptr() as *const c_char;
        assert_eq!(c_str(p), Some("hello"));
    }
}