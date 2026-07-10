// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Darwin (macOS) miscellaneous utilities.
//!
//! Idiomatic Rust translation of PCSX2's `CocoaToolsBridge` and related
//! Darwin helpers. Provides bundle path resolution, URL opening, and
//! Finder reveal functionality, implemented with `std` only. Non-macOS
//! targets receive empty stubs that return safe defaults so the module
//! can be referenced unconditionally from cross-platform code.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Returns the directory containing the running `.app` bundle — i.e. the
/// parent directory of the bundle itself.
#[cfg(target_os = "macos")]
pub fn get_bundle_directory() -> Option<PathBuf> {
    get_app_bundle_path()
        .and_then(|app| app.parent().map(|p| p.to_path_buf()))
}

/// Returns the bundle directory with macOS app translocation bypassed.
///
/// App translocation moves quarantined bundles into a private read-only
/// location and exposes a synthetic path back to the process. Resolving
/// the directory through `/usr/bin/realpath` returns the original on-disk
/// location when possible.
#[cfg(target_os = "macos")]
pub fn get_non_translocated_bundle_directory() -> Option<PathBuf> {
    realpath(&get_bundle_directory()?)
}

/// Returns `true` if the current executable is running from inside a
/// `.app` bundle.
#[cfg(target_os = "macos")]
pub fn is_bundle() -> bool {
    get_app_bundle_path().is_some()
}

/// Returns `~/Library/Caches/<App.app>` for the running bundle.
#[cfg(target_os = "macos")]
pub fn get_cache_directory() -> Option<PathBuf> {
    let app = get_app_bundle_path()?;
    let home = std::env::var_os("HOME")?;
    let mut path = PathBuf::from(home);
    path.push("Library");
    path.push("Caches");
    path.push(app.file_name()?);
    Some(path)
}

/// Returns the `Contents/Resources` directory of the running bundle.
#[cfg(target_os = "macos")]
pub fn get_resources_directory() -> Option<PathBuf> {
    let mut path = get_app_bundle_path()?;
    path.push("Contents");
    path.push("Resources");
    Some(path)
}

/// Returns the path of the running `.app` bundle.
#[cfg(target_os = "macos")]
pub fn get_app_bundle_path() -> Option<PathBuf> {
    find_app_bundle(&std::env::current_exe().ok()?)
}

/// Returns the path of the frontend `.app` bundle.
///
/// In a monolithic build the frontend shares a bundle with the running
/// process, so this is identical to [`get_app_bundle_path`].
#[cfg(target_os = "macos")]
pub fn get_frontend_bundle_path() -> Option<PathBuf> {
    get_app_bundle_path()
}

/// Opens the given URL in the user's default handler.
#[cfg(target_os = "macos")]
pub fn open_url(url: &str) -> bool {
    Command::new("open")
        .arg(url)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Reveals the given file or directory in Finder by selecting it in an
/// `open -R` invocation.
#[cfg(target_os = "macos")]
pub fn show_in_finder(path: &Path) -> bool {
    Command::new("open")
        .arg("-R")
        .arg(path)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Non-macOS stubs
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "macos"))]
pub fn get_bundle_directory() -> Option<PathBuf> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn get_non_translocated_bundle_directory() -> Option<PathBuf> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn is_bundle() -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn get_cache_directory() -> Option<PathBuf> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn get_resources_directory() -> Option<PathBuf> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn get_app_bundle_path() -> Option<PathBuf> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn get_frontend_bundle_path() -> Option<PathBuf> {
    None
}

#[cfg(not(target_os = "macos"))]
pub fn open_url(_url: &str) -> bool {
    false
}

#[cfg(not(target_os = "macos"))]
pub fn show_in_finder(_path: &Path) -> bool {
    false
}

// ---------------------------------------------------------------------------
// macOS helpers
// ---------------------------------------------------------------------------

/// Walks up from `exe` until a directory whose name has the `.app`
/// extension is found.
#[cfg(target_os = "macos")]
fn find_app_bundle(exe: &Path) -> Option<PathBuf> {
    let mut current: Option<&Path> = Some(exe);
    while let Some(p) = current {
        if p.extension().and_then(|e| e.to_str()) == Some("app") {
            return Some(p.to_path_buf());
        }
        current = p.parent();
    }
    None
}

/// Resolves `p` to its canonical on-disk location, returning `None` if
/// the system `realpath` utility is missing or fails.
#[cfg(target_os = "macos")]
fn realpath(p: &Path) -> Option<PathBuf> {
    let output = Command::new("/usr/bin/realpath").arg(p).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let s = std::str::from_utf8(&output.stdout).ok()?.trim();
    (!s.is_empty()).then(|| PathBuf::from(s))
}
