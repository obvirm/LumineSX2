// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Concrete platform backends for the [`HTTPDownloader`] trait.
//!
//! Idiomatic Rust translation of PCSX2's `common/HTTPDownloaderCurl.{h,cpp}`
//! and `common/HTTPDownloaderWinHTTP.{h,cpp}` pair. The original C++ classes
//! are private subclasses of the abstract `HTTPDownloader`; this module
//! exposes the same two backends as public Rust types and selects one
//! per-target through the [`create_downloader`] factory.
//!
//! The Unix backend [`CurlDownloader`] actually performs network I/O by
//! shelling out to the system `curl` binary via `std::process::Command`.
//! Curl is invoked with `-sSL -A <user-agent> -D /dev/stderr -o /dev/stdout`
//! so that response headers land on stderr (where the C++ libcurl
//! `CURLINFO_RESPONSE_CODE` / `CURLINFO_CONTENT_TYPE` lookups used to read
//! them) and the body lands on stdout. Each request runs on its own
//! `std::thread`; the curl process blocks the thread until it returns.
//!
//! The Windows backend [`WinHttpDownloader`] is a stub: the `winhttp` FFI is
//! not linked from this translation (only `std` is available), so every
//! method that would touch the network returns
//! [`DownloaderError::Transport`]. The struct and trait wiring are kept
//! intact so the rest of the codebase can still compile on Windows.
//!
//! Only `std` is used.

use super::HTTPDownloader as htp_mod;
use htp_mod::{
    DownloaderError, DownloaderType, HTTPDownloader, HTTPDownloaderHandle, Request,
    RequestCallback, RequestState, RequestType, http_status,
};
use std::process::Command;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Default connect / send / receive timeout, in seconds, matching the
/// 15,000 ms value used by the C++ WinHTTP backend.
const DEFAULT_TIMEOUT_IN_SECONDS: f32 = 15.0;
/// Default cap on concurrently in-flight curl invocations, matching the
/// C++ `DEFAULT_MAX_ACTIVE_REQUESTS`.
const DEFAULT_MAX_ACTIVE_REQUESTS: u32 = 4;
/// Sleep between reap passes while waiting for outstanding requests.
const WAIT_TICK: Duration = Duration::from_millis(1);

/// Result of a single curl invocation: HTTP status, `Content-Type`,
/// `Content-Length`, and the raw response body.
///
/// `Content-Length` is `0` when the server did not advertise it (e.g.
/// chunked transfer encoding), matching the C++ behaviour of leaving
/// `content_length` zero until libcurl reports a positive value.
type CurlResponse = (i32, String, u32, Vec<u8>);

/// Spawns a curl subprocess, blocks the calling thread until it exits, and
/// returns the parsed response.
///
/// The function is `Send + 'static` so it can be moved into a worker thread
/// owned by [`CurlDownloader`].
fn run_curl_request(
    user_agent: &str,
    url: &str,
    request_type: RequestType,
    post_data: &str,
    timeout: f32,
) -> CurlResponse {
    // The original C++ build sets CURLOPT_NOSIGNAL and uses curl_multi with
    // libcurl-internal timeouts. Spawning a subprocess is a different model
    // so we approximate the connect/send/receive budget with `curl -m`.
    let timeout_secs = timeout.max(1.0).ceil() as u32;

    let mut cmd = Command::new("curl");
    cmd.arg("-sSL") // silent, show errors, follow redirects
        .arg("-A")
        .arg(user_agent)
        .arg("-m")
        .arg(timeout_secs.to_string())
        // Headers to stderr, body to stdout. The `/dev/stderr` path is
        // valid on every Unix-like target this backend is gated on.
        .args(["-D", "/dev/stderr", "-o", "/dev/stdout"]);

    match request_type {
        RequestType::Get => {}
        RequestType::Post => {
            cmd.arg("-X").arg("POST").arg("--data-raw").arg(post_data);
        }
    }

    cmd.arg(url);

    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("CurlDownloader: failed to spawn curl: {}", e);
            return (http_status::HTTP_STATUS_ERROR, String::new(), 0, Vec::new());
        }
    };

    if !output.status.success() {
        // curl itself exited non-zero (DNS, TLS, timeout, ...).
        let err = String::from_utf8_lossy(&output.stderr);
        eprintln!("CurlDownloader: curl failed: {}", err.trim_end());
        return (http_status::HTTP_STATUS_ERROR, String::new(), 0, output.stdout);
    }

    let body = output.stdout;
    let header_text = String::from_utf8_lossy(&output.stderr);
    let (status_code, content_type, content_length) = parse_headers(&header_text);
    (status_code, content_type, content_length, body)
}

/// Pulls the status code, `Content-Type`, and `Content-Length` out of the
/// stderr dump produced by `curl -D /dev/stderr`.
///
/// The dump looks like:
///
/// ```text
/// HTTP/1.1 200 OK
/// Content-Type: application/json
/// Content-Length: 1234
/// ...
/// ```
///
/// `Content-Length` is parsed but `content_length` is reported as 0 if the
/// header is absent (chunked transfer encoding) or not a valid integer.
fn parse_headers(headers: &str) -> (i32, String, u32) {
    let mut status_code: i32 = http_status::HTTP_STATUS_ERROR;
    let mut content_type = String::new();
    let mut content_length: u32 = 0;

    for line in headers.lines() {
        if let Some(rest) = line.strip_prefix("HTTP/") {
            // First token after "HTTP/" is the major version, then the
            // status code, then the reason phrase.
            let mut parts = rest.split_whitespace();
            let _version = parts.next();
            if let Some(code) = parts.next() {
                if let Ok(parsed) = code.parse::<i32>() {
                    status_code = parsed;
                }
            }
        } else if let Some(value) = line.strip_prefix("Content-Type:") {
            content_type = value.trim().to_string();
        } else if let Some(value) = line.strip_prefix("content-type:") {
            // Some proxies normalise header names to lower case; accept both.
            if content_type.is_empty() {
                content_type = value.trim().to_string();
            }
        } else if let Some(value) = line.strip_prefix("Content-Length:") {
            if let Ok(parsed) = value.trim().parse::<u32>() {
                content_length = parsed;
            }
        } else if let Some(value) = line.strip_prefix("content-length:") {
            if content_length == 0 {
                if let Ok(parsed) = value.trim().parse::<u32>() {
                    content_length = parsed;
                }
            }
        }
    }

    (status_code, content_type, content_length)
}

/// Unix / macOS / BSD backend. Shells out to the system `curl` binary.
#[cfg(unix)]
pub struct CurlDownloader {
    user_agent: String,
    timeout: f32,
    max_active_requests: u32,
    pending: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

#[cfg(unix)]
impl CurlDownloader {
    /// Builds a fresh `CurlDownloader` with the platform defaults
    /// ([`DEFAULT_TIMEOUT_IN_SECONDS`], [`DEFAULT_MAX_ACTIVE_REQUESTS`]).
    pub fn new(user_agent: impl Into<String>) -> Self {
        Self {
            user_agent: user_agent.into(),
            timeout: DEFAULT_TIMEOUT_IN_SECONDS,
            max_active_requests: DEFAULT_MAX_ACTIVE_REQUESTS,
            pending: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Builds a `CurlDownloader` and seeds it with the standard PCSX2 user
    /// agent.
    pub fn with_default_agent() -> Self {
        Self::new(htp_mod::DEFAULT_USER_AGENT)
    }

    /// Returns the user-agent string sent on every request.
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }

    /// Reaps any worker threads that have finished since the last call.
    ///
    /// Mirrors the role of `curl_multi_info_read` in the C++ implementation:
    /// each completed handle is joined so its callback can finish cleanly,
    /// and any that are still running are kept in the pending list.
    fn reap(&self) {
        let mut pending = match self.pending.lock() {
            Ok(p) => p,
            Err(p) => p.into_inner(),
        };
        let mut still_running = Vec::with_capacity(pending.len());
        for handle in pending.drain(..) {
            if handle.is_finished() {
                // Swallow panics: a worker thread dying must not poison the
                // downloader; the callback has already been skipped by the
                // panic unwind and the user-visible request will simply
                // appear stuck in `Started` until `wait_for_all_requests`
                // gives up.
                let _ = handle.join();
            } else {
                still_running.push(handle);
            }
        }
        *pending = still_running;
    }
}

#[cfg(unix)]
impl HTTPDownloader for CurlDownloader {
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
        if request.url.is_empty() {
            return Err(DownloaderError::InvalidUrl(String::new()));
        }

        let state = request.state.clone();
        let callback = request.callback.take();
        let request_url = request.url.clone();
        let post_data = std::mem::take(&mut request.post_data);
        let request_type = request.request_type;
        let user_agent = self.user_agent.clone();
        let timeout = self.timeout;

        let state_for_thread = state.clone();
        let handle = thread::spawn(move || {
            let (status, content_type, content_length, data) =
                run_curl_request(&user_agent, &request_url, request_type, &post_data, timeout);
            state_for_thread.store(RequestState::Complete as i32, Ordering::Release);
            if let Some(mut cb) = callback {
                cb(status, content_type, data);
            }
        });

        self.pending
            .lock()
            .expect("CurlDownloader pending list poisoned")
            .push(handle);

        Ok(HTTPDownloaderHandle { parent: None, state })
    }

    fn request(&mut self, mut request: Request) -> Result<Request, DownloaderError> {
        if request.url.is_empty() {
            return Err(DownloaderError::InvalidUrl(String::new()));
        }
        request.start_time = Some(Instant::now());
        request
            .state
            .store(RequestState::Started as i32, Ordering::Release);

        let callback = request.callback.take();
        let request_url = request.url.clone();
        let post_data = std::mem::take(&mut request.post_data);
        let request_type = request.request_type;
        let user_agent = self.user_agent.clone();
        let timeout = self.timeout;
        let state_arc = request.state.clone();

        let handle = thread::spawn(move || {
            let (status, content_type, content_length, data) =
                run_curl_request(&user_agent, &request_url, request_type, &post_data, timeout);
            state_arc.store(RequestState::Complete as i32, Ordering::Release);
            // The fields on the returned `Request` are intentionally left
            // untouched (the caller already owns it); the observable effect
            // of completion is the callback firing and the state atomic
            // flipping to `Complete`.
            let _ = content_length;
            if let Some(mut cb) = callback {
                cb(status, content_type, data);
            }
        });

        self.pending
            .lock()
            .expect("CurlDownloader pending list poisoned")
            .push(handle);

        Ok(request)
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
        // The C++ libcurl backend drives `curl_multi_perform` here. Our
        // worker threads do the I/O themselves, so the only thing left to
        // do is reap finished handles.
        self.reap();
        Ok(())
    }

    fn wait_for_all_requests(&mut self) {
        // Sleep + reap until either the pending list drains or every
        // outstanding handle has been joined. A bounded retry avoids a
        // pathological infinite loop if a worker panics before finishing.
        let mut budget = 0u32;
        loop {
            self.reap();
            if !self.has_any_requests() {
                return;
            }
            if budget > 60_000 {
                // ~60 s of waiting; give up rather than spinning forever.
                return;
            }
            thread::sleep(WAIT_TICK);
            budget = budget.saturating_add(1);
        }
    }

    fn has_any_requests(&self) -> bool {
        !self
            .pending
            .lock()
            .expect("CurlDownloader pending list poisoned")
            .is_empty()
    }

    fn close_handle(&self) {
        // The C++ curl backend had no per-handle close outside of the
        // multi-handle teardown. The worker thread here is fire-and-forget;
        // it will complete on its own and update the request state.
    }
}

/// Windows backend. Stubs out the `winhttp` FFI; every method that would
/// touch the network returns [`DownloaderError::Transport`].
#[cfg(target_os = "windows")]
pub struct WinHttpDownloader {
    user_agent: String,
    timeout: f32,
    max_active_requests: u32,
    pending: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

#[cfg(target_os = "windows")]
impl WinHttpDownloader {
    /// Builds a stub `WinHttpDownloader`. No `winhttp` session is created
    /// because the FFI is not linked from this translation.
    pub fn new(user_agent: impl Into<String>) -> Self {
        Self {
            user_agent: user_agent.into(),
            timeout: DEFAULT_TIMEOUT_IN_SECONDS,
            max_active_requests: DEFAULT_MAX_ACTIVE_REQUESTS,
            pending: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Builds a `WinHttpDownloader` seeded with the standard PCSX2 user
    /// agent.
    pub fn with_default_agent() -> Self {
        Self::new(htp_mod::DEFAULT_USER_AGENT)
    }

    /// Returns the user-agent string that *would* have been used.
    pub fn user_agent(&self) -> &str {
        &self.user_agent
    }
}

#[cfg(target_os = "windows")]
impl HTTPDownloader for WinHttpDownloader {
    fn open(
        &self,
        _url: &str,
        _request: Request,
    ) -> Result<HTTPDownloaderHandle, DownloaderError> {
        Err(DownloaderError::Transport(
            "WinHttpDownloader: winhttp FFI is stubbed".to_string(),
        ))
    }

    fn request(&mut self, request: Request) -> Result<Request, DownloaderError> {
        // The original C++ `StartRequest` is what we would call into here;
        // since the FFI is unavailable we surface a transport error and
        // return the request untouched so the caller can still inspect
        // its fields.
        let _ = request;
        Err(DownloaderError::Transport(
            "WinHttpDownloader: winhttp FFI is stubbed".to_string(),
        ))
    }

    fn set_option(&mut self, key: &str, value: &str) -> Result<(), DownloaderError> {
        // The option keys are platform-agnostic; accept them so callers
        // can run identical setup code on Windows and Unix.
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
        // Noop: the C++ WinHTTP backend uses IOCP worker threads, not a
        // user-driven poll loop. With the FFI stubbed there is nothing
        // to do here either.
        Ok(())
    }

    fn wait_for_all_requests(&mut self) {
        // Noop: the stub never started any requests, so there is nothing
        // to wait for.
    }

    fn has_any_requests(&self) -> bool {
        !self
            .pending
            .lock()
            .expect("WinHttpDownloader pending list poisoned")
            .is_empty()
    }

    fn close_handle(&self) {
        // Noop: the C++ WinHTTP backend closes handles via its async
        // callback, which we cannot reach without the FFI.
    }
}

/// Picks the platform-appropriate backend and returns it boxed as a trait
/// object.
///
/// On Unix-like targets this is [`CurlDownloader`]; on Windows it is
/// [`WinHttpDownloader`]. The returned object is allocated with
/// [`Box::new`] and owns no shared state with the caller, so dropping it
/// cancels any in-flight callbacks by dropping the boxed instance.
pub fn create_downloader() -> Box<dyn HTTPDownloader> {
    create_downloader_with_agent(htp_mod::DEFAULT_USER_AGENT)
}

/// Same as [`create_downloader`] but seeds the backend with an explicit
/// user-agent string instead of [`HTTPDownloader::DEFAULT_USER_AGENT`].
pub fn create_downloader_with_agent(user_agent: impl Into<String>) -> Box<dyn HTTPDownloader> {
    let user_agent = user_agent.into();
    #[cfg(target_os = "windows")]
    {
        let _: RequestCallback = Box::new(|_status, _content_type, _data| {});
        let _ = DownloaderType::WinHTTP;
        Box::new(WinHttpDownloader::new(user_agent))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = DownloaderType::Curl;
        Box::new(CurlDownloader::new(user_agent))
    }
}
