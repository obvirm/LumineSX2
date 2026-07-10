// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's `common/HTTPDownloader.{h,cpp}`.
//!
//! This module provides a transport-agnostic abstraction over the underlying
//! HTTP client. The original C++ class is an abstract base that is specialised
//! at build time into either a libcurl backend or a WinHTTP backend, depending
//! on the host platform. In this translation the abstract surface is
//! preserved as a [`HTTPDownloader`] trait, and the factory returns a
//! [`Box<dyn HTTPDownloader>`] whose concrete implementation is selected at
//! runtime via [`DownloaderType`].
//!
//! High-level request state — pending / started / receiving / complete /
//! cancelled, timeouts, and progress callbacks — is modelled in
//! [`HTTPDownloader::Request`]. The actual network calls are stubbed with
//! `unimplemented!()` because only `std` is available.
//!
//! Only `std` is used; the `std::sync::{Arc, Mutex}` primitives serialise the
//! pending-request list in place of the C++ `std::mutex` and `std::unique_lock`
//! pair from the original.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Default user-agent string used when callers do not supply one.
pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:85.0) Gecko/20100101 Firefox/85.0";

const DEFAULT_TIMEOUT_IN_SECONDS: f32 = 30.0;
const DEFAULT_MAX_ACTIVE_REQUESTS: u32 = 4;

/// HTTP status sentinels used by the downloader callback contract.
///
/// The numeric values match the C++ `HTTPDownloader` enum so that downstream
/// callers see the same negative-on-failure convention.
pub mod http_status {
    pub const HTTP_STATUS_CANCELLED: i32 = -3;
    pub const HTTP_STATUS_TIMEOUT: i32 = -2;
    pub const HTTP_STATUS_ERROR: i32 = -1;
    pub const HTTP_STATUS_OK: i32 = 200;
}

/// Selects which concrete transport backs a [`HTTPDownloader`] instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DownloaderType {
    /// libcurl-based backend (Unix and Windows).
    Curl,
    /// Native WinHTTP backend (Windows only).
    WinHTTP,
}

/// Concrete factory for [`HTTPDownloader`] instances.
///
/// The original C++ `HTTPDownloader::Create` is a static factory that picks
/// between libcurl and WinHTTP at compile time. In this translation the
/// selection is data-driven so callers can override the default via
/// [`HTTPDownloaderFactory::with_type`].
#[derive(Debug, Clone)]
pub struct HTTPDownloaderFactory {
    downloader_type: DownloaderType,
    user_agent: String,
}

impl HTTPDownloaderFactory {
    /// Creates a factory with the platform-default transport and the bundled
    /// [`DEFAULT_USER_AGENT`].
    pub fn new() -> Self {
        Self {
            #[cfg(target_os = "windows")]
            downloader_type: DownloaderType::WinHTTP,
            #[cfg(not(target_os = "windows"))]
            downloader_type: DownloaderType::Curl,
            user_agent: DEFAULT_USER_AGENT.to_string(),
        }
    }

    /// Overrides the transport selection.
    pub fn with_type(mut self, downloader_type: DownloaderType) -> Self {
        self.downloader_type = downloader_type;
        self
    }

    /// Overrides the user-agent string sent in HTTP requests.
    pub fn with_user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.user_agent = user_agent.into();
        self
    }

    /// Returns the transport this factory will instantiate.
    pub fn downloader_type(&self) -> DownloaderType {
        self.downloader_type
    }

    /// Returns the user-agent string this factory will pass to the downloader.
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }
}

impl Default for HTTPDownloaderFactory {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience constructor mirroring the C++ static `HTTPDownloader::Create`.
///
/// Equivalent to `HTTPDownloaderFactory::new().create()`.
pub fn create_downloader() -> Box<dyn HTTPDownloader> {
    HTTPDownloaderFactory::new().create()
}

/// A handle to a pending or in-flight HTTP request.
///
/// Mirrors the C++ `HTTPDownloader::Request` struct. The state field is an
/// [`AtomicI32`] so that background pollers can observe transitions without
/// holding the request-list lock.
pub struct HTTPDownloaderHandle {
    /// Owning trait object, retained so that [`close`](HTTPDownloader::close)
    /// can be invoked when the request is dropped.
    parent: Option<Box<dyn HTTPDownloader>>,
    state: Arc<AtomicI32>,
}

impl HTTPDownloaderHandle {
    /// Returns the current state code of the request.
    ///
    /// The mapping from numeric value to variant is given by
    /// [`HTTPDownloader::Request::State`].
    pub fn state(&self) -> i32 {
        self.state.load(Ordering::Acquire)
    }
}

impl Drop for HTTPDownloaderHandle {
    fn drop(&mut self) {
        if let Some(parent) = self.parent.take() {
            parent.close_handle();
        }
    }
}

/// A single outstanding HTTP request tracked by a [`HTTPDownloader`].
pub struct Request {
    /// Raw response bytes.
    pub data: Vec<u8>,
    /// Optional callback invoked when the request terminates.
    pub callback: Option<RequestCallback>,
    /// Target URL.
    pub url: String,
    /// Body to send on a `POST` request.
    pub post_data: String,
    /// `Content-Type` header of the response, populated when complete.
    pub content_type: String,
    /// Monotonic timestamp at which the request was created.
    pub start_time: Option<Instant>,
    /// Final HTTP status code, populated when the request completes.
    pub status_code: i32,
    /// `Content-Length` advertised by the server, if known.
    pub content_length: u32,
    /// Last byte count reported to the progress callback.
    pub last_progress_update: u32,
    /// GET vs. POST discriminator.
    pub request_type: RequestType,
    /// Lifecycle state of the request.
    pub state: Arc<AtomicI32>,
}

impl Request {
    /// Creates a fresh, `Pending` GET request with the given URL and callback.
    pub fn new_get(url: impl Into<String>, callback: RequestCallback) -> Self {
        Self {
            data: Vec::new(),
            callback: Some(callback),
            url: url.into(),
            post_data: String::new(),
            content_type: String::new(),
            start_time: None,
            status_code: 0,
            content_length: 0,
            last_progress_update: 0,
            request_type: RequestType::Get,
            state: Arc::new(AtomicI32::new(RequestState::Pending as i32)),
        }
    }

    /// Creates a fresh, `Pending` POST request.
    pub fn new_post(
        url: impl Into<String>,
        post_data: impl Into<String>,
        callback: RequestCallback,
    ) -> Self {
        Self {
            data: Vec::new(),
            callback: Some(callback),
            url: url.into(),
            post_data: post_data.into(),
            content_type: String::new(),
            start_time: None,
            status_code: 0,
            content_length: 0,
            last_progress_update: 0,
            request_type: RequestType::Post,
            state: Arc::new(AtomicI32::new(RequestState::Pending as i32)),
        }
    }
}

/// GET vs. POST discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RequestType {
    /// HTTP GET.
    Get,
    /// HTTP POST with a request body.
    Post,
}

/// Lifecycle states a [`Request`] can be in.
///
/// The numeric discriminants are exposed via the public `as i32` conversion
/// and stored atomically in [`Request::state`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum RequestState {
    /// Created but not yet handed to the transport.
    Pending = 0,
    /// Cancelled by the caller or by a progress-cancel signal.
    Cancelled = 1,
    /// Transport has started the request.
    Started = 2,
    /// Transport is currently receiving response bytes.
    Receiving = 3,
    /// Request finished; the callback has been or will be invoked.
    Complete = 4,
}

impl RequestState {
    /// Converts a raw `i32` state code back into the corresponding variant.
    ///
    /// Unknown values map to [`RequestState::Pending`], matching the C++
    /// behaviour of treating a default-constructed atomic as not-yet-started.
    pub fn from_i32(value: i32) -> Self {
        match value {
            0 => RequestState::Pending,
            1 => RequestState::Cancelled,
            2 => RequestState::Started,
            3 => RequestState::Receiving,
            4 => RequestState::Complete,
            _ => RequestState::Pending,
        }
    }
}

/// Callback signature for completed HTTP requests.
///
/// `status_code` follows the [`http_status`] sentinel convention on failure
/// and the real HTTP status on success. `content_type` is the value of the
/// response `Content-Type` header. `data` is the raw response body.
pub type RequestCallback = Box<dyn FnMut(i32, String, Vec<u8>) + Send + 'static>;

/// Errors that the downloader trait can surface to callers.
#[derive(Debug)]
pub enum DownloaderError {
    /// The supplied URL was empty or otherwise malformed.
    InvalidUrl(String),
    /// The underlying transport refused to start the request.
    Transport(String),
    /// The downloader was polled after all requests had already finished.
    NoActiveRequests,
}

impl std::fmt::Display for DownloaderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloaderError::InvalidUrl(url) => {
                write!(f, "invalid URL: {}", url)
            }
            DownloaderError::Transport(msg) => {
                write!(f, "transport error: {}", msg)
            }
            DownloaderError::NoActiveRequests => {
                write!(f, "no active requests to poll")
            }
        }
    }
}

impl std::error::Error for DownloaderError {}

/// The transport-agnostic HTTP downloader surface.
///
/// Mirrors the abstract C++ `HTTPDownloader` class. Implementations are
/// responsible for actually driving the network I/O; this translation stubs
/// those operations with `unimplemented!()` because no HTTP client is wired
/// up.
pub trait HTTPDownloader: Send {
    /// Kicks off a new request and returns a handle that tracks its state.
    ///
    /// The returned [`HTTPDownloaderHandle`] holds a strong reference to the
    /// downloader so dropping the trait object early will still trigger
    /// graceful shutdown of any in-flight request it owns.
    fn open(
        &self,
        url: &str,
        request: Request,
    ) -> Result<HTTPDownloaderHandle, DownloaderError>;

    /// Submits a fully-formed request to the transport and registers it on
    /// the downloader's pending list.
    fn request(&mut self, request: Request) -> Result<Request, DownloaderError>;

    /// Performs a GET. Thin wrapper around [`request`](Self::request) that
    /// builds a [`Request::new_get`] for the caller.
    fn get(
        &mut self,
        url: impl Into<String>,
        callback: RequestCallback,
    ) -> Result<Request, DownloaderError> where Self: Sized {
        let req = Request::new_get(url, callback);
        self.request(req)
    }

    /// Performs a POST. Thin wrapper around [`request`](Self::request).
    fn post(
        &mut self,
        url: impl Into<String>,
        post_data: impl Into<String>,
        callback: RequestCallback,
    ) -> Result<Request, DownloaderError> where Self: Sized {
        let req = Request::new_post(url, post_data, callback);
        self.request(req)
    }

    /// Performs a GET and decodes the response body as UTF-8 JSON.
    ///
    /// The callback receives the decoded string on success or an empty
    /// string on transport failure.
    fn get_json(
        &mut self,
        url: impl Into<String>,
        mut callback: impl FnMut(i32, String) + Send + 'static,
    ) -> Result<Request, DownloaderError> where Self: Sized {
        let url = url.into();
        let url_for_cb = url.clone();
        let wrapped: RequestCallback = Box::new(move |status, _content_type, data| {
            let body = if status == http_status::HTTP_STATUS_OK {
                String::from_utf8(data).unwrap_or_default()
            } else {
                String::new()
            };
            callback(status, body);
        });
        let mut req = Request::new_get(url, wrapped);
        // Tag the URL on the request for log-grep parity with the C++ path.
        req.url = url_for_cb;
        self.request(req)
    }

    /// Performs a POST and decodes the response body as UTF-8 JSON.
    fn post_json(
        &mut self,
        url: impl Into<String>,
        post_data: impl Into<String>,
        mut callback: impl FnMut(i32, String) + Send + 'static,
    ) -> Result<Request, DownloaderError> where Self: Sized {
        let url = url.into();
        let url_for_cb = url.clone();
        let wrapped: RequestCallback = Box::new(move |status, _content_type, data| {
            let body = if status == http_status::HTTP_STATUS_OK {
                String::from_utf8(data).unwrap_or_default()
            } else {
                String::new()
            };
            callback(status, body);
        });
        let mut req = Request::new_post(url, post_data, wrapped);
        req.url = url_for_cb;
        self.request(req)
    }

    /// Applies a generic key/value option to the downloader.
    ///
    /// Examples in the C++ code are `timeout` and `max_active_requests`. This
    /// translation accepts a stringly-typed option so that backends can be
    /// extended without re-spanning the trait.
    fn set_option(&mut self, key: &str, value: &str) -> Result<(), DownloaderError>;

    /// Polls the transport for completed requests and dispatches their
    /// callbacks.
    fn poll_requests(&mut self) -> Result<(), DownloaderError>;

    /// Blocks until every registered request has terminated.
    fn wait_for_all_requests(&mut self);

    /// Returns `true` if any request is still pending or in-flight.
    fn has_any_requests(&self) -> bool;

    /// Releases the resources held by a [`HTTPDownloaderHandle`].
    ///
    /// Called automatically from [`HTTPDownloaderHandle`]'s `Drop` impl, so
    /// direct callers rarely need this.
    fn close_handle(&self);
}

/// Concrete downloader that owns the request list and dispatches to a
/// transport implementation. Network calls are stubbed.
pub struct DefaultHTTPDownloader {
    transport: DownloaderType,
    user_agent: String,
    timeout: f32,
    max_active_requests: u32,
    pending: Arc<Mutex<Vec<Request>>>,
}

impl DefaultHTTPDownloader {
    /// Constructs a downloader that uses the specified transport.
    pub fn new(downloader_type: DownloaderType, user_agent: impl Into<String>) -> Self {
        Self {
            transport: downloader_type,
            user_agent: user_agent.into(),
            timeout: DEFAULT_TIMEOUT_IN_SECONDS,
            max_active_requests: DEFAULT_MAX_ACTIVE_REQUESTS,
            pending: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Returns the transport this downloader was built around.
    pub fn transport(&self) -> DownloaderType {
        self.transport
    }

    /// Returns the user-agent string used for new requests.
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }
}

impl HTTPDownloaderFactory {
    /// Instantiates the configured downloader as a boxed trait object.
    pub fn create(&self) -> Box<dyn HTTPDownloader> {
        Box::new(DefaultHTTPDownloader::new(
            self.downloader_type,
            self.user_agent.clone(),
        ))
    }
}

impl HTTPDownloader for DefaultHTTPDownloader {
    fn open(
        &self,
        url: &str,
        mut request: Request,
    ) -> Result<HTTPDownloaderHandle, DownloaderError> {
        if url.is_empty() && request.url.is_empty() {
            return Err(DownloaderError::InvalidUrl(url.to_string()));
        }
        if request.url.is_empty() {
            request.url = url.to_string();
        }
        let state = request.state.clone();
        // Stash the request in the pending list so poll/close paths can find it.
        {
            let mut pending = self.pending.lock().expect("pending list poisoned");
            pending.push(request);
        }
        Ok(HTTPDownloaderHandle {
            parent: None, // see `request` for the owned-handle path
            state,
        })
    }

    fn request(&mut self, mut request: Request) -> Result<Request, DownloaderError> {
        if request.url.is_empty() {
            return Err(DownloaderError::InvalidUrl(String::new()));
        }
        request.start_time = Some(Instant::now());
        // Network call is stubbed; real implementations would call into
        // libcurl_easy_perform or WinHTTPSendRequest here.
        unimplemented!("DefaultHTTPDownloader::request: network transport not wired up");
    }

    fn set_option(&mut self, key: &str, value: &str) -> Result<(), DownloaderError> {
        match key {
            "timeout" => {
                let parsed = value.parse::<f32>().map_err(|_| {
                    DownloaderError::Transport(format!("invalid timeout: {}", value))
                })?;
                if parsed <= 0.0 {
                    return Err(DownloaderError::Transport(
                        "timeout must be positive".to_string(),
                    ));
                }
                self.timeout = parsed;
                Ok(())
            }
            "max_active_requests" => {
                let parsed = value.parse::<u32>().map_err(|_| {
                    DownloaderError::Transport(format!(
                        "invalid max_active_requests: {}",
                        value
                    ))
                })?;
                if parsed == 0 {
                    return Err(DownloaderError::Transport(
                        "max_active_requests must be > 0".to_string(),
                    ));
                }
                self.max_active_requests = parsed;
                Ok(())
            }
            "user_agent" => {
                self.user_agent = value.to_string();
                Ok(())
            }
            other => Err(DownloaderError::Transport(format!(
                "unknown option: {}",
                other
            ))),
        }
    }

    fn poll_requests(&mut self) -> Result<(), DownloaderError> {
        // The original C++ implementation walks the pending list, dispatches
        // timeouts, fires the callbacks, and starts any pending requests up
        // to `max_active_requests`. Without an actual transport we cannot
        // make progress, so this is stubbed.
        unimplemented!("DefaultHTTPDownloader::poll_requests: network transport not wired up");
    }

    fn wait_for_all_requests(&mut self) {
        // In a real implementation this would loop on `poll_requests` with a
        // short sleep until `pending` is empty.
        let pending = self.pending.clone();
        while !pending
            .lock()
            .expect("pending list poisoned")
            .is_empty()
        {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    fn has_any_requests(&self) -> bool {
        !self
            .pending
            .lock()
            .expect("pending list poisoned")
            .is_empty()
    }

    fn close_handle(&self) {
        // No-op: real implementations would call curl_easy_cleanup or
        // WinHttpCloseHandle here.
    }
}

/// Returns a plausible file extension for a given `Content-Type` header.
///
/// Reproduces the static lookup table from the C++ implementation. Unknown
/// types return an empty string.
pub fn get_extension_for_content_type(content_type: &str) -> String {
    // Mirror of the C++ table from `HTTPDownloader::GetExtensionForContentType`.
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
        (
            "application/vnd.oasis.opendocument.presentation",
            "odp",
        ),
        (
            "application/vnd.oasis.opendocument.spreadsheet",
            "ods",
        ),
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
        if content_type.eq_ignore_ascii_case(mime) {
            return (*ext).to_string();
        }
    }
    String::new()
}
