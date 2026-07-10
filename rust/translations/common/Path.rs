// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Cross-platform path helpers translated from PCSX2's `common/Path.h`.
//!
//! These functions provide idiomatic Rust equivalents for locating the running
//! executable, the bundled content/resources directories, and joining path
//! components together. Only the `std` crate is used.

use std::path::{Path, PathBuf};

/// Returns the directory containing the currently running executable.
///
/// Uses [`std::env::current_exe`] and falls back to `None` if the OS does not
/// report an executable path (e.g. on platforms where it is unsupported).
pub fn exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf))
}

/// Returns the directory used for user-modifiable content (saves, memcards,
/// BIOS dumps, etc.).
///
/// On desktop platforms this is typically the executable directory or a
/// `Content/` subdirectory beside it. On non-supported platforms this returns
/// `None`.
pub fn content_dir() -> Option<PathBuf> {
    let override_dir = std::env::var("PCSX2_CONTENT_DIR").ok().map(PathBuf::from);
    if let Some(dir) = override_dir {
        return Some(dir);
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    {
        exe_path().map(|p| p.join("Content"))
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// Returns the directory containing the bundled resources (shaders, locales,
/// documentation, etc.).
///
/// On desktop platforms this is typically the executable directory or a
/// `Resources/` subdirectory beside it. On non-supported platforms this
/// returns `None`.
pub fn resources_dir() -> Option<PathBuf> {
    let override_dir = std::env::var("PCSX2_RESOURCES_DIR").ok().map(PathBuf::from);
    if let Some(dir) = override_dir {
        return Some(dir);
    }

    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    {
        exe_path().map(|p| p.join("Resources"))
    }

    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// Joins an arbitrary number of path components into a single [`PathBuf`].
///
/// Behaves like `Path::join` applied repeatedly: a later component that is
/// absolute replaces everything that came before it. An empty `parts` slice
/// yields an empty [`PathBuf`].
pub fn combine(parts: &[&str]) -> PathBuf {
    let mut result = PathBuf::new();
    for part in parts {
        if part.is_empty() {
            continue;
        }
        result.push(part);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combine_empty_returns_empty() {
        assert_eq!(combine(&[]), PathBuf::new());
    }

    #[test]
    fn combine_joins_components() {
        assert_eq!(
            combine(&["foo", "bar", "baz.txt"]),
            PathBuf::from("foo").join("bar").join("baz.txt")
        );
    }

    #[test]
    fn combine_skips_empty_parts() {
        assert_eq!(combine(&["foo", "", "bar"]), PathBuf::from("foo").join("bar"));
    }

    #[test]
    fn combine_absolute_resets() {
        // Path::join with an absolute root resets the base.
        assert_eq!(combine(&["foo", "/bar", "baz"]), PathBuf::from("/bar/baz"));
    }
}
