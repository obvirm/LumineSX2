// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's `common/RedtapeWindows.h`.
//!
//! The original C/C++ header centralises the Windows-specific macro hygiene
//! that must be set *before* `<Windows.h>` is included, and then pulls the
//! core Win32 header in. In Rust we get the `windows` crate (or the
//! `Win32` API) without those macro hazards, so this module is mostly a
//! documentation shim: it pins the same platform target (Windows 10 or
//! later) and exposes the equivalent compile-time constants so that
//! downstream code can refer to them symbolically.
//!
//! On non-Windows targets the module is a no-op stub so that `mod` and
//! `use` statements in portable code keep working.

#![allow(dead_code)]

/// Marker constant indicating that the `WIN32_LEAN_AND_MEAN` policy from
/// the original header is in effect: only the lean set of Win32 headers
/// should be pulled in. Provided for parity with the C++ macro.
#[cfg(target_os = "windows")]
pub const WIN32_LEAN_AND_MEAN: () = ();

/// Marker constant indicating that the `NOMINMAX` policy from the
/// original header is in effect: the `min` and `max` Win32 macros are
/// suppressed so they do not collide with `std::min`/`std::max` (or in
/// our case, with `std::cmp::min`/`max`). Provided for parity with the
/// C++ macro.
#[cfg(target_os = "windows")]
pub const NOMINMAX: () = ();

/// Minimum supported Windows version, matching the original header's
/// `_WIN32_WINNT 0x0A00` (Windows 10). Exposed as a `u32` so callers can
/// compare against it without re-encoding the literal.
#[cfg(target_os = "windows")]
pub const _WIN32_WINNT: u32 = 0x0A00;

/// Human-readable name of the minimum supported Windows version, for
/// diagnostics and documentation.
#[cfg(target_os = "windows")]
pub const MIN_WINDOWS_VERSION_NAME: &str = "Windows 10";

/// Returns `true` when the build target is Windows.
///
/// The original header is conditionally compiled only when `_WIN32` is
/// defined; this helper mirrors that check.
#[cfg(target_os = "windows")]
#[inline]
pub const fn is_windows() -> bool {
    true
}

/// Stub used on non-Windows targets so cross-platform code can
/// reference the same symbols uniformly.
#[cfg(not(target_os = "windows"))]
#[inline]
pub const fn is_windows() -> bool {
    false
}

/// Stub: `WIN32_LEAN_AND_MEAN` is a no-op outside of the Win32 build.
#[cfg(not(target_os = "windows"))]
pub const WIN32_LEAN_AND_MEAN: () = ();

/// Stub: `NOMINMAX` is a no-op outside of the Win32 build.
#[cfg(not(target_os = "windows"))]
pub const NOMINMAX: () = ();

/// Stub: the minimum-Windows-version constant is meaningless off Windows.
#[cfg(not(target_os = "windows"))]
pub const _WIN32_WINNT: u32 = 0;

/// Stub: name of the minimum supported Windows version off Windows.
#[cfg(not(target_os = "windows"))]
pub const MIN_WINDOWS_VERSION_NAME: &str = "";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_flag_matches_target() {
        assert_eq!(is_windows(), cfg!(target_os = "windows"));
    }

    #[test]
    fn min_version_is_windows_10() {
        assert_eq!(_WIN32_WINNT, 0x0A00);
        assert_eq!(MIN_WINDOWS_VERSION_NAME, "Windows 10");
    }
}
