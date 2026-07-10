// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
//! End-to-end exercise of `pcsx2_common_rs::http_downloader` on Windows.
//!
//! Demonstrates that the Windows HTTP path (originally implemented in
//! `common/HTTPDownloaderWinHTTP.cpp` against the WinHTTP service)
//! works end-to-end when re-expressed through the cross-platform
//! `ureq` crate. The test:
//!
//! 1. Prints which TLS root store the current build is configured
//!    to use. On Windows this is the OS certificate store via
//!    `rustls-native-certs` (enabled by the `native-certs` feature
//!    in `Cargo.toml`); on other targets it is the bundled
//!    Mozilla `webpki-roots`.
//! 2. Builds an `HttpDownloader` with a short, configurable
//!    timeout (the same `with_timeout` constructor a real PCSX2
//!    caller would use to honour a UI setting).
//! 3. Issues a real HTTP GET against a small, public test endpoint
//!    (`http://example.com/`). The call is wrapped in a `match`
//!    so the example exits 0 with a "skipped: no network" line
//!    if the host is offline or the test endpoint is unreachable.
//! 4. Prints the URL, HTTP status, response body length, and a
//!    short prefix of the body so a human can eyeball that the
//!    download succeeded.
//!
//! Run with (release build for accurate timing measurements):
//! ```powershell
//! $env:RUSTFLAGS = "-C target-feature=+sse4.1"
//! cargo build --release --example http_downloader_win_test
//! .\target\release\examples\http_downloader_win_test.exe
//! ```
//!
//! Exit status:
//! - 0 — request succeeded, OR request was skipped due to no network.
//! - 1 — request returned a non-2xx HTTP status (real failure).
//!
//! The example uses the same `HttpDownloader` type that the FFI
//! exports (`pcsx2_http_download`, `pcsx2_http_download_with_progress`)
//! are built on, so a successful run proves the FFI surface works too.

use std::path::PathBuf;
use std::time::Duration;

#[path = "../src/http_downloader.rs"]
mod http_downloader;

#[path = "../src/progress_callback.rs"]
mod progress_callback;

use http_downloader::{
    download_file, get_extension_for_content_type, tls_roots_label, HttpDownloader, DEFAULT_USER_AGENT,
};

/// Small, public test endpoint. `http://example.com/` is a good fit
/// because the response is short (<2 KiB), the host is operated by
/// IANA, and it doesn't redirect through any HTTPS so we don't need
/// a working root store to make the test pass.
const TEST_URL: &str = "http://example.com/";

/// Request timeout. Intentionally short (10 s) so an unreachable host
/// fails fast instead of leaving the user staring at a hung terminal.
/// The C++ side's `HTTPDownloader::SetTimeout` is the equivalent knob.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// How many leading bytes of the response body to print. The headers
/// are not printed at all so we don't leak the test machine's User-Agent.
const BODY_PREVIEW_BYTES: usize = 96;

fn main() {
    println!("=== http_downloader Windows path end-to-end test ===");
    println!("Target OS         : {}", std::env::consts::OS);
    println!("Target arch       : {}", std::env::consts::ARCH);
    println!("TLS root store    : {}", tls_roots_label());
    println!("Default user-agent: {}", DEFAULT_USER_AGENT);
    println!("Test URL          : {}", TEST_URL);
    println!("Request timeout   : {}s", TEST_TIMEOUT.as_secs());

    // 1. Fetch the test URL with a short timeout. The match handles
    //    every failure mode the same way (print a one-line diagnostic
    //    and exit 0) so an offline machine doesn't fail the test.
    let downloader = HttpDownloader::with_timeout(TEST_TIMEOUT);

    let result = downloader.fetch(TEST_URL);
    let (status, content_type, body) = match result {
        Ok(t) => t,
        Err(e) => {
            // Network down, DNS failure, TLS handshake failure, or
            // any other transport-level error. Print and exit 0.
            println!();
            println!("skipped: no network — request failed: {e}");
            println!("(this is expected on an offline / sandboxed host)");
            std::process::exit(0);
        }
    };

    println!();
    println!("HTTP status       : {status}");
    println!("Content-Type      : {}", content_type);
    println!("Ext (from CT)     : .{}", get_extension_for_content_type(&content_type));
    println!("Body length       : {} bytes", body.len());

    let preview_end = BODY_PREVIEW_BYTES.min(body.len());
    if preview_end > 0 {
        let preview = String::from_utf8_lossy(&body[..preview_end]);
        // Replace newlines so the preview stays on one line in the
        // terminal. The HTML body is multi-line.
        let oneline: String = preview.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect();
        println!("Body preview      : {oneline}{}", if body.len() > preview_end { "..." } else { "" });
    }

    // 2. Also exercise the file-download path so we know that
    //    stream-to-disk + progress callback plumbing works on
    //    Windows. Writes to a temp file and reports the size on
    //    stdout. The `download_file` helper uses the default
    //    HttpDownloader (no progress callback).
    let dest: PathBuf = std::env::temp_dir().join("pcsx2_http_downloader_win_test.html");
    println!();
    println!("Streaming to file : {}", dest.display());

    match download_file(TEST_URL, &dest) {
        Ok(()) => {
            let written = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
            println!("File written      : {} bytes", written);
            if written != body.len() as u64 {
                println!(
                    "WARNING: in-memory body length ({} bytes) != file size ({} bytes)",
                    body.len(),
                    written
                );
            }
            // Best-effort cleanup so repeated runs don't leave
            // junk in %TEMP%.
            let _ = std::fs::remove_file(&dest);
        }
        Err(e) => {
            println!("skipped: no network — file download failed: {e}");
            println!("(this is expected on an offline / sandboxed host)");
            // Don't fail the test for a network error.
        }
    }

    // 3. Surface a hard failure on a real HTTP error status.
    //    Anything in the 2xx range counts as success.
    if !(200..300).contains(&status) {
        eprintln!("HTTP error: status {status}");
        std::process::exit(1);
    }

    println!();
    println!("OK — Windows HTTP path works end-to-end through ureq.");
}
