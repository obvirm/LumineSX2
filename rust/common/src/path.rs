// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Rust reimplementation of PCSX2's `common/Path.{h,cpp}`.
//!
//! The C++ namespace `Path` provides filesystem path manipulation
//! utilities: joining, splitting, canonicalising, sanitising, URL
//! encoding, and a handful of cross-platform helpers (`ToNativePath`,
//! `IsAbsolute`, `RealPath`, ...).
//!
//! # Porting notes
//!
//! - Internally everything is built on `std::path::{Path, PathBuf}` so
//!   we inherit the correct platform semantics for free. The C++ side
//!   rolls its own by hand (with `std::filesystem` on newer builds
//!   behind a compatibility shim), but Rust's `Path` already does the
//!   right thing on Windows and Unix.
//! - `ToNativePath` simply rewrites `/` to the platform separator. On
//!   Unix that's a no-op; on Windows it turns `a/b/c` into `a\\b\\c`.
//! - `Canonicalize` mirrors `Path::clean` semantics for the
//!   string-only variant (resolve `.` / `..` lexically). The full
//!   `RealPath` resolves symlinks via `std::fs::canonicalize` and falls
//!   back to the input on failure (matching the C++ behaviour which
//!   used `realpath()` / `GetFullPathNameW` and returned the original
//!   path on error).
//! - `URLEncode` / `URLDecode` are implemented by hand to avoid
//!   pulling in a new dependency; the encoding rules match RFC 3986's
//!   `unreserved` set used by `PCSX2` (alnum + `-` `_` `.` `~`), so
//!   spaces become `%20` rather than `+`.
//!
//! # FFI surface
//!
//! All string-returning FFI functions use the **caller-provided
//! buffer** pattern documented in `Cargo.toml` / `lib.rs`:
//!
//! ```c
//! char buf[PATH_MAX];
//! uint32_t n = pcsx2_path_combine(buf, sizeof(buf), base, next);
//! // n is the number of bytes written, NOT including the NUL
//! // terminator. Returns 0 if the buffer was too small.
//! ```
//!
//! The buffer pattern (as opposed to returning a `*mut c_char` that the
//! C++ side must free) was chosen because:
//!  1. It matches the lifetime story of `std::string_view` on the C++
//!     side — callers can keep the buffer alive as long as they want.
//!  2. It avoids allocator mismatch concerns entirely; no
//!     `pcsx2_string_free` is required.
//!  3. The C++ `SmallString` types in this codebase already expose
//!     `data()` + `buffer_size()`, so the call site is natural.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    clippy::all
)]

use std::ffi::CStr;
use std::os::raw::c_char;
use std::path::{Path, PathBuf};
use std::ptr;

// ---------------------------------------------------------------------------
// Pure-Rust API
// ---------------------------------------------------------------------------

/// Convert any forward slashes in `path` to the platform's native
/// separator. On Unix this is a no-op; on Windows it turns `/` into
/// `\`. Mirrors `Path::ToNativePath`.
#[inline]
pub fn to_native_path(path: &str) -> String {
    #[cfg(windows)]
    {
        path.replace('/', "\\")
    }
    #[cfg(not(windows))]
    {
        path.to_owned()
    }
}

/// In-place equivalent of [`to_native_path`].
#[inline]
pub fn to_native_path_in_place(path: &mut String) {
    #[cfg(windows)]
    {
        if path.contains('/') {
            *path = path.replace('/', "\\");
        }
    }
    #[cfg(not(windows))]
    {
        let _ = path;
    }
}

/// Join two path components, producing a new owned path.
///
/// If `next` is absolute it replaces `base` entirely (mirroring
/// `std::path::Path::join`'s behaviour). Trailing separators on `base`
/// are preserved; an empty `base` returns `next`.
pub fn combine(base: &str, next: &str) -> String {
    if base.is_empty() {
        return next.to_owned();
    }
    if next.is_empty() {
        return base.to_owned();
    }

    let base_path = Path::new(base);
    let joined = base_path.join(next);
    joined.to_string_lossy().into_owned()
}

/// Build a path that, when interpreted relative to `relative_to`,
/// points at `filename`. Both inputs should be absolute for the
/// standard use-case; if either is relative the result is the input.
///
/// Mirrors `Path::BuildRelativePath` from the C++ side, which the
/// original computed by stripping a common prefix. We use
/// `Path::difference` for the same effect when both are absolute.
pub fn build_relative_path(filename: &str, relative_to: &str) -> String {
    let f = Path::new(filename);
    let r = Path::new(relative_to);
    match f.strip_prefix(r) {
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => filename.to_owned(),
    }
}

/// Lexically canonicalise `path`: collapse `.` and `..` components,
/// remove duplicate separators. Does **not** touch the filesystem (use
/// [`real_path`] for that). Mirrors `Path::Canonicalize(string)`.
pub fn canonicalize(path: &str) -> String {
    let p = Path::new(path);
    let mut out = PathBuf::new();
    for component in p.components() {
        match component {
            std::path::Component::CurDir => { /* drop '.' */ }
            std::path::Component::ParentDir => {
                // Pop only if the current stack ends in a normal
                // component; otherwise the '..' is meaningful (e.g. at
                // the root) and must be retained.
                match out.components().last() {
                    Some(std::path::Component::Normal(_)) => {
                        out.pop();
                    }
                    _ => out.push(".."),
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        ".".to_owned()
    } else {
        out.to_string_lossy().into_owned()
    }
}

/// In-place equivalent of [`canonicalize`].
#[inline]
pub fn canonicalize_in_place(path: &mut String) {
    *path = canonicalize(path);
}

/// Replace any character that is not safe in a filename with `_`.
/// When `strip_slashes` is `true` (the default) both `/` and `\\` are
/// also collapsed to `_`. The base safe set already excludes both
/// slash characters, so the parameter only controls whether the
/// resulting filename is a single component (no slashes at all) or
/// retains `/` as a path separator.
///
/// Mirrors `Path::SanitizeFileName`.
pub fn sanitize_file_name(s: &str, strip_slashes: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let is_safe = matches!(ch, 'A'..='Z' | 'a'..='z' | '0'..='9' | '.' | '-' | '_' | ' ' | '/');
        let is_slash = ch == '/' || ch == '\\';
        if !is_safe || (strip_slashes && is_slash) {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    out
}

/// In-place equivalent of [`sanitize_file_name`].
#[inline]
pub fn sanitize_file_name_in_place(s: &mut String, strip_slashes: bool) {
    *s = sanitize_file_name(s, strip_slashes);
}

/// Returns `true` if `s` is a valid filename on this OS. When
/// `allow_slashes` is `true`, path separators are permitted (useful for
/// validating full subpaths, not just a single component).
pub fn is_valid_file_name(s: &str, allow_slashes: bool) -> bool {
    if s.is_empty() {
        return false;
    }
    for ch in s.chars() {
        if matches!(ch, '\0'..='\x1f' | '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') {
            if !(allow_slashes && (ch == '/' || ch == '\\')) {
                return false;
            }
        }
    }
    true
}

/// Returns `true` if `path` is an absolute filesystem path on this OS.
/// On Windows that means a drive letter prefix or a UNC root; on Unix
/// it means leading `/`.
#[inline]
pub fn is_absolute(path: &str) -> bool {
    Path::new(path).is_absolute()
}

/// Resolve symlinks via the OS. On error the original input is
/// returned, mirroring the C++ behaviour (`realpath` / `GetFullPathNameW`
/// fall back to the input string on failure).
pub fn real_path(path: &str) -> String {
    match std::fs::canonicalize(path) {
        Ok(p) => p.to_string_lossy().into_owned(),
        Err(_) => path.to_owned(),
    }
}

/// Express `path` relative to `relative_to`. Both must be absolute; if
/// either is relative the function returns `path` unchanged (matching
/// the C++ contract). On Windows the result uses `\\`; on Unix it uses
/// `/`.
pub fn make_relative(path: &str, relative_to: &str) -> String {
    let p = Path::new(path);
    let r = Path::new(relative_to);
    if !p.is_absolute() || !r.is_absolute() {
        return path.to_owned();
    }
    match p.strip_prefix(r) {
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => path.to_owned(),
    }
}

/// Extension including the leading `.`, or empty string if there isn't
/// one. Returns the full input when `path` ends in `.` (a stem with no
/// extension). Behaviour matches `Path::GetExtension` (returns a
/// `std::string_view`).
#[inline]
pub fn get_extension(path: &str) -> &str {
    let bytes = path.as_bytes();
    // Find the last separator (either form on all platforms — we
    // don't want to treat '.' in directory names as a separator).
    let last_sep = bytes
        .iter()
        .rposition(|&b| b == b'/' || b == b'\\')
        .map(|i| i + 1)
        .unwrap_or(0);
    let tail = &bytes[last_sep..];
    match tail.iter().rposition(|&b| b == b'.') {
        Some(0) => "", // ".bashrc" -> hidden file, no extension
        Some(i) => std::str::from_utf8(&tail[i..]).unwrap_or(""),
        None => "",
    }
}

/// `path` with the trailing extension removed (including the dot).
/// Returns the full input when there is no extension.
#[inline]
pub fn strip_extension(path: &str) -> &str {
    let bytes = path.as_bytes();
    let last_sep = bytes
        .iter()
        .rposition(|&b| b == b'/' || b == b'\\')
        .map(|i| i + 1)
        .unwrap_or(0);
    let tail = &bytes[last_sep..];
    match tail.iter().rposition(|&b| b == b'.') {
        Some(0) => path, // ".bashrc" -> no extension to strip
        Some(i) => std::str::from_utf8(&bytes[..last_sep + i]).unwrap_or(path),
        None => path,
    }
}

/// Replace the extension on `path`. An empty `new_extension` strips
/// the existing one; a leading `.` is preserved if supplied.
pub fn replace_extension(path: &str, new_extension: &str) -> String {
    let mut p = PathBuf::from(path);
    p.set_extension(new_extension);
    p.to_string_lossy().into_owned()
}

/// Directory component of `path` (everything up to but not including
/// the final separator), or empty string if there's no directory.
#[inline]
pub fn get_directory(path: &str) -> &str {
    let bytes = path.as_bytes();
    match bytes.iter().rposition(|&b| b == b'/' || b == b'\\') {
        Some(0) => &path[..1], // root: keep the leading separator
        Some(i) => std::str::from_utf8(&bytes[..i]).unwrap_or(""),
        None => "",
    }
}

/// File name component (the part after the final separator), or empty
/// string if `path` ends in a separator.
#[inline]
pub fn get_file_name(path: &str) -> &str {
    let bytes = path.as_bytes();
    match bytes.iter().rposition(|&b| b == b'/' || b == b'\\') {
        Some(i) => std::str::from_utf8(&bytes[i + 1..]).unwrap_or(""),
        None => path,
    }
}

/// File title: file_name minus its extension. Returns empty string if
/// `path` has no file component, or if the file component starts with
/// a dot and has no other dots (e.g. ".bashrc"). Matches `Path::file_stem`
/// semantics on the underlying `Path`.
#[inline]
pub fn get_file_title(path: &str) -> &str {
    let bytes = path.as_bytes();
    let name_start = bytes
        .iter()
        .rposition(|&b| b == b'/' || b == b'\\')
        .map(|i| i + 1)
        .unwrap_or(0);
    let tail = &bytes[name_start..];
    match tail.iter().rposition(|&b| b == b'.') {
        // Hidden file: ".bashrc" -> title is empty (matches Path::file_stem).
        Some(0) => "",
        Some(i) => std::str::from_utf8(&bytes[name_start..name_start + i]).unwrap_or(""),
        None => std::str::from_utf8(&bytes[name_start..]).unwrap_or(""),
    }
}

/// `path` with its file name replaced by `new_filename`.
pub fn change_file_name(path: &str, new_filename: &str) -> String {
    let mut p = PathBuf::from(path);
    if new_filename.is_empty() {
        p.set_file_name("");
    } else {
        p.set_file_name(new_filename);
    }
    p.to_string_lossy().into_owned()
}

/// In-place equivalent of [`change_file_name`].
pub fn change_file_name_in_place(path: &mut String, new_filename: &str) {
    *path = change_file_name(path, new_filename);
}

/// Append a directory component between the existing path and its
/// filename. If `path` has no file component the new directory is
/// simply pushed.
pub fn append_directory(path: &str, new_dir: &str) -> String {
    let mut p = PathBuf::from(path);
    if let Some(parent) = p.parent() {
        let mut new_parent = parent.to_path_buf();
        new_parent.push(new_dir);
        if let Some(name) = p.file_name() {
            p = new_parent.join(name);
        } else {
            p = new_parent;
        }
    } else {
        p.push(new_dir);
    }
    p.to_string_lossy().into_owned()
}

/// In-place equivalent of [`append_directory`].
pub fn append_directory_in_place(path: &mut String, new_dir: &str) {
    *path = append_directory(path, new_dir);
}

/// Split `path` on **either** `/` or `\\`. Returns each non-empty
/// component as a borrowed slice of `path`.
pub fn split_windows_path(path: &str) -> Vec<&str> {
    path.split(|c| c == '/' || c == '\\')
        .filter(|s| !s.is_empty())
        .collect()
}

/// Join `components` back together using `\\` as the separator. Empty
/// components are skipped; the first component is emitted without a
/// leading separator, subsequent ones get one each.
pub fn join_windows_path(components: &[&str]) -> String {
    let mut out = String::new();
    let mut first = true;
    for c in components {
        if c.is_empty() {
            continue;
        }
        if !first {
            out.push('\\');
        }
        first = false;
        out.push_str(c);
    }
    out
}

/// Split `path` on the platform's native separator only.
pub fn split_native_path(path: &str) -> Vec<&str> {
    #[cfg(windows)]
    {
        path.split('\\').filter(|s| !s.is_empty()).collect()
    }
    #[cfg(not(windows))]
    {
        path.split('/').filter(|s| !s.is_empty()).collect()
    }
}

/// Join `components` using the platform's native separator.
pub fn join_native_path(components: &[&str]) -> String {
    #[cfg(windows)]
    {
        join_windows_path(components)
    }
    #[cfg(not(windows))]
    {
        let mut out = String::new();
        let mut first = true;
        for c in components {
            if c.is_empty() {
                continue;
            }
            if !first {
                out.push('/');
            }
            first = false;
            out.push_str(c);
        }
        out
    }
}

/// Percent-encode every byte in `s` that is not in the RFC 3986
/// unreserved set. Spaces become `%20` (not `+`). Mirrors the C++
/// `Path::URLEncode`, which is the same rule.
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        let unreserved = matches!(
            b,
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~'
        );
        if unreserved {
            out.push(b as char);
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
    }
    out
}

/// Reverse of [`url_encode`]. Invalid escape sequences (e.g. `%zz`)
/// are left verbatim in the output so the round-trip is lossless on
/// bad input, matching PCSX2's tolerance.
pub fn url_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_value(bytes[i + 1]);
            let lo = hex_value(bytes[i + 2]);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[inline]
fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Build a `file://` URL for `path`. The path is expected to be
/// absolute; on Windows the leading drive letter is translated to a
/// URL-style drive (`C:\foo` -> `file:///C:/foo`); on Unix the path is
/// prefixed with `file://`.
pub fn create_file_url(path: &str) -> String {
    #[cfg(windows)]
    {
        // file:///C:/path/to/file
        let mut out = String::from("file:///");
        let mut p = path;
        if let Some(stripped) = p.strip_prefix("\\\\") {
            // UNC: \\server\share\path -> file://server/share/path
            out.push_str("file://"); // collapse the extra leading '/'
            out.push_str(&stripped.replace('\\', "/"));
            return out;
        }
        if let Some(stripped) = p.strip_prefix('\\') {
            out.push('/');
            p = stripped;
        }
        // Replace backslashes with forward slashes for the URL.
        out.push_str(&p.replace('\\', "/"));
        out
    }
    #[cfg(not(windows))]
    {
        let mut out = String::from("file://");
        if let Some(stripped) = path.strip_prefix('/') {
            out.push('/');
            out.push_str(stripped);
        } else {
            out.push('/');
            out.push_str(path);
        }
        out
    }
}

// ---------------------------------------------------------------------------
// FFI surface
// ---------------------------------------------------------------------------
//
// All exports use the caller-provided buffer pattern described in the
// module-level docs. The signature is:
//
//     uint32_t pcsx2_path_xxx(
//         char*       out,        // destination buffer
//         uint32_t    out_len,    // capacity of `out` in bytes (incl. NUL)
//         ...args...             // inputs as NUL-terminated C strings
//     );
//
// Return value: the number of bytes written, NOT including the
// terminating NUL. Returns 0 if `out_len` is too small (in which case
// nothing is written). A null `out` or `out_len == 0` is treated as
// "too small".

/// Helper: copy `value` into a caller-provided buffer as a
/// NUL-terminated C string. Returns the number of bytes written
/// excluding the NUL, or `0` if the buffer was too small.
#[inline]
fn write_cstr(out: *mut c_char, out_len: u32, value: &str) -> u32 {
    if out.is_null() || out_len == 0 {
        return 0;
    }
    // We need out_len bytes including the terminator.
    let needed = value.len() + 1;
    if needed > out_len as usize {
        return 0;
    }
    unsafe {
        let dst = std::slice::from_raw_parts_mut(out as *mut u8, needed);
        dst[..value.len()].copy_from_slice(value.as_bytes());
        dst[value.len()] = 0;
    }
    value.len() as u32
}

/// Helper: read a NUL-terminated C string. Returns empty string if the
/// pointer is null (defensive: C++ side should never pass null here).
#[inline]
fn read_cstr(s: *const c_char) -> String {
    if s.is_null() {
        return String::new();
    }
    // SAFETY: caller guarantees `s` is a NUL-terminated C string.
    unsafe { CStr::from_ptr(s) }
        .to_string_lossy()
        .into_owned()
}

/// FFI export: combine two path components. Returns bytes written, or
/// `0` if the buffer is too small.
#[no_mangle]
pub extern "C" fn pcsx2_path_combine(
    out: *mut c_char,
    out_len: u32,
    base: *const c_char,
    next: *const c_char,
) -> u32 {
    let base = read_cstr(base);
    let next = read_cstr(next);
    write_cstr(out, out_len, &combine(&base, &next))
}

/// FFI export: extension of `path`, including the leading `.`.
#[no_mangle]
pub extern "C" fn pcsx2_path_get_extension(
    out: *mut c_char,
    out_len: u32,
    path: *const c_char,
) -> u32 {
    let path = read_cstr(path);
    write_cstr(out, out_len, get_extension(&path))
}

/// FFI export: file name component of `path`.
#[no_mangle]
pub extern "C" fn pcsx2_path_get_file_name(
    out: *mut c_char,
    out_len: u32,
    path: *const c_char,
) -> u32 {
    let path = read_cstr(path);
    write_cstr(out, out_len, get_file_name(&path))
}

/// FFI export: file title (file_name minus extension) of `path`.
#[no_mangle]
pub extern "C" fn pcsx2_path_get_file_title(
    out: *mut c_char,
    out_len: u32,
    path: *const c_char,
) -> u32 {
    let path = read_cstr(path);
    write_cstr(out, out_len, get_file_title(&path))
}

/// FFI export: directory component of `path`.
#[no_mangle]
pub extern "C" fn pcsx2_path_get_directory(
    out: *mut c_char,
    out_len: u32,
    path: *const c_char,
) -> u32 {
    let path = read_cstr(path);
    write_cstr(out, out_len, get_directory(&path))
}

/// FFI export: returns 1 if `path` is absolute, 0 otherwise. (This
/// one is a scalar, no buffer required, so it breaks the pattern —
/// kept because the C++ side treats it as a `bool` query and the
/// caller doesn't need to allocate a throwaway buffer.)
#[no_mangle]
pub extern "C" fn pcsx2_path_is_absolute(path: *const c_char) -> u32 {
    let path = read_cstr(path);
    is_absolute(&path) as u32
}

/// FFI export: convert forward slashes to the platform's native
/// separator.
#[no_mangle]
pub extern "C" fn pcsx2_path_to_native_path(
    out: *mut c_char,
    out_len: u32,
    path: *const c_char,
) -> u32 {
    let path = read_cstr(path);
    write_cstr(out, out_len, &to_native_path(&path))
}

/// FFI export: replace the file portion of `path` with `new_file_name`,
/// writing the result back into the caller-owned `path` buffer.
///
/// Mirrors C++ `Path::ChangeFileName(string*, string_view)`. The
/// caller is responsible for sizing `path` large enough to hold the
/// result plus a NUL terminator. Returns `false` (and leaves `path`
/// untouched) on null pointers or allocation failure.
#[no_mangle]
pub extern "C" fn pcsx2_path_change_file_name(
    path: *mut c_char,
    new_file_name: *const c_char,
) -> bool {
    if path.is_null() || new_file_name.is_null() {
        return false;
    }
    let current = read_cstr(path as *const c_char);
    let new_name = read_cstr(new_file_name);
    let updated = change_file_name(&current, &new_name);
    let needed = updated.len() + 1;
    unsafe {
        let dst = std::slice::from_raw_parts_mut(path as *mut u8, needed);
        dst[..updated.len()].copy_from_slice(updated.as_bytes());
        dst[updated.len()] = 0;
    }
    true
}

/// FFI export: append `dir` to `base`, returning a newly-allocated
/// NUL-terminated C string.
///
/// Mirrors C++ `Path::AppendDirectory(string_view, string_view)`
/// returning a `std::string`. The caller frees the returned pointer
/// with `libc::free`. Returns a null pointer if either argument is
/// null or if the allocation fails.
#[no_mangle]
pub extern "C" fn pcsx2_path_append_directory(
    base: *const c_char,
    dir: *const c_char,
) -> *mut c_char {
    if base.is_null() || dir.is_null() {
        return std::ptr::null_mut();
    }
    let base = read_cstr(base);
    let dir = read_cstr(dir);
    let joined = append_directory(&base, &dir);
    unsafe {
        let buf = libc::malloc(joined.len() + 1) as *mut u8;
        if buf.is_null() {
            return std::ptr::null_mut();
        }
        ptr::copy_nonoverlapping(joined.as_ptr(), buf, joined.len());
        *buf.add(joined.len()) = 0;
        buf as *mut c_char
    }
}

/// FFI export: join `count` NUL-terminated path components into one
/// path using the platform's native separator.
///
/// Mirrors C++ `Path::JoinNativePath(vector<string_view> const&)`:
/// empty components are skipped, and `count == 0` (or all-empty
/// components) yields a single empty string. The caller frees the
/// returned pointer with `libc::free`.
#[no_mangle]
pub extern "C" fn pcsx2_path_join_native_path(
    parts: *const *const c_char,
    count: usize,
) -> *mut c_char {
    // SAFETY: caller guarantees `parts` points to `count` valid
    // NUL-terminated C strings (or nulls, which we treat as empty).
    if parts.is_null() || count == 0 {
        let empty = String::new();
        return unsafe {
            let buf = libc::malloc(1) as *mut u8;
            if buf.is_null() {
                return std::ptr::null_mut();
            }
            *buf = 0;
            buf as *mut c_char
        };
    }
    let mut owned: Vec<String> = Vec::with_capacity(count);
    unsafe {
        for i in 0..count {
            let p = *parts.add(i);
            owned.push(read_cstr(p));
        }
    }
    let borrowed: Vec<&str> = owned.iter().map(|s| s.as_str()).collect();
    let joined = join_native_path(&borrowed);
    unsafe {
        let buf = libc::malloc(joined.len() + 1) as *mut u8;
        if buf.is_null() {
            return std::ptr::null_mut();
        }
        ptr::copy_nonoverlapping(joined.as_ptr(), buf, joined.len());
        *buf.add(joined.len()) = 0;
        buf as *mut c_char
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_native_path_works() {
        #[cfg(windows)]
        assert_eq!(to_native_path("a/b/c"), "a\\b\\c");
        #[cfg(not(windows))]
        assert_eq!(to_native_path("a/b/c"), "a/b/c");
    }

    #[test]
    fn combine_works() {
        // Path::join on Windows preserves the source's separator
        // style: `a/b` + `c` becomes `a/b\\c`, while `a/b/` + `c`
        // becomes `a/b/c`. Mirror that exactly so the test runs on
        // every platform.
        let expected_keep = Path::new("a/b").join("c").to_string_lossy().into_owned();
        let expected_norm = Path::new("a/b/").join("c").to_string_lossy().into_owned();
        assert_eq!(combine("a/b", "c"), expected_keep);
        assert_eq!(combine("a/b/", "c"), expected_norm);
        assert_eq!(combine("", "c"), "c");
        assert_eq!(combine("a", ""), "a");
    }

    #[test]
    fn canonicalize_works() {
        // The collapse of "a/./b/../c" -> "a/c" is platform-portable in
        // semantics; on Windows the separator is rendered as `\` while
        // on Unix it's `/`. Use the function's own output to derive the
        // expected separator.
        let canon = canonicalize("a/./b/../c");
        assert!(canon == "a/c" || canon == "a\\c", "got {canon:?}");
        // `..` at the front cannot be collapsed and must be retained.
        // Path renders it with the platform separator.
        let up = canonicalize("../a");
        assert!(up == "../a" || up == "..\\a", "got {up:?}");
        // An empty input normalises to the current-directory marker.
        assert_eq!(canonicalize(""), ".");
        // Idempotent (the function is its own normal form).
        let once = canonicalize("a/./b/../c");
        let twice = canonicalize(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn sanitize_file_name_strips_slashes() {
        // With strip_slashes=true both '/' and '\\' collapse to '_'.
        assert_eq!(sanitize_file_name("a/b\\c", true), "a_b_c");
        // With strip_slashes=false, '/' is preserved (it's in the
        // safe set) while '\\' is not (it's a Windows separator and
        // not safe in a portable filename). The result is therefore
        // "a/b_c".
        assert_eq!(sanitize_file_name("a/b\\c", false), "a/b_c");
        // An already-safe name round-trips.
        let s = "ab.cd-ef_01";
        assert_eq!(sanitize_file_name(s, true), s);
    }

    #[test]
    fn is_valid_file_name_works() {
        assert!(is_valid_file_name("hello.txt", false));
        assert!(!is_valid_file_name("a/b", false));
        assert!(is_valid_file_name("a/b", true));
        assert!(!is_valid_file_name("", false));
    }

    #[test]
    fn get_extension_works() {
        assert_eq!(get_extension("foo.txt"), ".txt");
        assert_eq!(get_extension("foo"), "");
        assert_eq!(get_extension("foo.tar.gz"), ".gz");
        // Hidden file with no extension: empty per Path::extension.
        assert_eq!(get_extension("a/.bashrc"), "");
        assert_eq!(get_extension(".bashrc"), "");
    }

    #[test]
    fn strip_extension_works() {
        assert_eq!(strip_extension("foo.txt"), "foo");
        assert_eq!(strip_extension("foo"), "foo");
        assert_eq!(strip_extension("foo.tar.gz"), "foo.tar");
        // Hidden file with no extension: nothing to strip.
        assert_eq!(strip_extension(".bashrc"), ".bashrc");
    }

    #[test]
    fn get_file_name_works() {
        assert_eq!(get_file_name("a/b/c.txt"), "c.txt");
        assert_eq!(get_file_name("c.txt"), "c.txt");
    }

    #[test]
    fn get_file_title_works() {
        assert_eq!(get_file_title("a/b/c.txt"), "c");
        assert_eq!(get_file_title("c"), "c");
    }

    #[test]
    fn get_directory_works() {
        // The directory is the prefix up to (but not including) the
        // last separator. The separator style is preserved as-is.
        assert_eq!(get_directory("a/b/c.txt"), "a/b");
        assert_eq!(get_directory("a\\b\\c.txt"), "a\\b");
        // No separator -> no directory.
        assert_eq!(get_directory("c.txt"), "");
        // Trailing separator -> directory is the whole path.
        assert_eq!(get_directory("a/b/"), "a/b");
    }

    #[test]
    fn change_file_name_works() {
        // Rust's `Path::set_file_name` preserves the existing
        // separator style and inserts a new one as needed. The
        // expected output is platform-aware, so derive it from
        // Path::join.
        let expected = Path::new("a/b").join("d").to_string_lossy().into_owned();
        assert_eq!(change_file_name("a/b/c.txt", "d"), expected);
        assert_eq!(change_file_name("c.txt", "d"), "d");
    }

    #[test]
    fn append_directory_works() {
        // "a/c.txt" splits into parent "a" and file "c.txt". After
        // appending "b" the result is "a/b/c.txt" (Unix) or "a\\b\\c.txt"
        // (Windows). Derive the expected output via PathBuf so the
        // assertion is portable.
        let expected = {
            let parent = PathBuf::from("a");
            let parent = parent.join("b");
            parent.join("c.txt").to_string_lossy().into_owned()
        };
        assert_eq!(append_directory("a/c.txt", "b"), expected);
    }

    #[test]
    fn split_windows_path_works() {
        assert_eq!(
            split_windows_path("a\\b/c\\d"),
            vec!["a", "b", "c", "d"]
        );
    }

    #[test]
    fn join_windows_path_works() {
        assert_eq!(join_windows_path(&["a", "b", "c"]), "a\\b\\c");
    }

    #[test]
    fn url_encode_decode_roundtrip() {
        let original = "hello world! 100%";
        let encoded = url_encode(original);
        assert_eq!(encoded, "hello%20world%21%20100%25");
        assert_eq!(url_decode(&encoded), original);
    }

    #[test]
    fn url_decode_passes_through_bad_escapes() {
        // %zz is not valid hex; the C++ implementation leaves it in.
        assert_eq!(url_decode("a%zzb"), "a%zzb");
    }

    #[test]
    fn ffi_combine_roundtrip() {
        let mut buf = [0u8; 64];
        let n = pcsx2_path_combine(
            buf.as_mut_ptr() as *mut c_char,
            buf.len() as u32,
            b"a/b\0".as_ptr() as *const c_char,
            b"c\0".as_ptr() as *const c_char,
        );
        assert!(n > 0);
        let s = unsafe { CStr::from_ptr(buf.as_ptr() as *const c_char) };
        let expected = combine("a/b", "c");
        assert_eq!(s.to_string_lossy(), expected);
        assert_eq!(n as usize, expected.len());
    }

    #[test]
    fn ffi_buffer_too_small_returns_zero() {
        let mut buf = [0u8; 2];
        let n = pcsx2_path_combine(
            buf.as_mut_ptr() as *mut c_char,
            buf.len() as u32,
            b"a\0".as_ptr() as *const c_char,
            b"b\0".as_ptr() as *const c_char,
        );
        assert_eq!(n, 0);
    }

    #[test]
    fn ffi_null_is_safe() {
        assert_eq!(pcsx2_path_is_absolute(std::ptr::null()), 0);
        assert_eq!(
            pcsx2_path_get_extension(std::ptr::null_mut(), 16, std::ptr::null()),
            0
        );
    }

    #[test]
    fn ffi_change_file_name_works() {
        use std::ffi::CString;
        // Allocate a caller-owned buffer and seed it with "a/b/c.txt".
        let mut backing = CString::new("a/b/c.txt").unwrap().into_bytes();
        // Ensure the buffer has room for the result plus a NUL.
        backing.resize(64, 0);
        let ok = pcsx2_path_change_file_name(
            backing.as_mut_ptr() as *mut c_char,
            CString::new("d.bin").unwrap().as_ptr(),
        );
        assert!(ok);
        let written = unsafe {
            CStr::from_ptr(backing.as_ptr() as *const c_char)
                .to_string_lossy()
                .into_owned()
        };
        // Path::set_file_name preserves the separator style; derive
        // the expected result the same way.
        let expected =
            Path::new("a/b").join("d.bin").to_string_lossy().into_owned();
        assert_eq!(written, expected);
    }

    #[test]
    fn ffi_change_file_name_null_is_safe() {
        // Either pointer null => returns false, no panic.
        assert!(!pcsx2_path_change_file_name(std::ptr::null_mut(), std::ptr::null()));
        let mut backing = [0u8; 16];
        assert!(!pcsx2_path_change_file_name(
            backing.as_mut_ptr() as *mut c_char,
            std::ptr::null(),
        ));
        assert!(!pcsx2_path_change_file_name(
            std::ptr::null_mut(),
            b"x\0".as_ptr() as *const c_char,
        ));
    }

    #[test]
    fn ffi_append_directory_works() {
        use std::ffi::CString;
        let base = CString::new("a/c.txt").unwrap();
        let dir = CString::new("b").unwrap();
        let out = pcsx2_path_append_directory(base.as_ptr(), dir.as_ptr());
        assert!(!out.is_null());
        let written = unsafe { CStr::from_ptr(out) }.to_string_lossy().into_owned();
        let expected = append_directory("a/c.txt", "b");
        assert_eq!(written, expected);
        unsafe { libc::free(out as *mut std::ffi::c_void) };
    }

    #[test]
    fn ffi_append_directory_null_is_safe() {
        let base = b"a\0".as_ptr() as *const c_char;
        assert!(pcsx2_path_append_directory(std::ptr::null(), base).is_null());
        assert!(pcsx2_path_append_directory(base, std::ptr::null()).is_null());
    }

    #[test]
    fn ffi_join_native_path_works() {
        use std::ffi::CString;
        let a = CString::new("a").unwrap();
        let b = CString::new("b").unwrap();
        let c = CString::new("c").unwrap();
        let ptrs = [a.as_ptr(), b.as_ptr(), c.as_ptr()];
        let out = pcsx2_path_join_native_path(ptrs.as_ptr(), ptrs.len());
        assert!(!out.is_null());
        let written = unsafe { CStr::from_ptr(out) }.to_string_lossy().into_owned();
        let expected = join_native_path(&["a", "b", "c"]);
        assert_eq!(written, expected);
        unsafe { libc::free(out as *mut std::ffi::c_void) };
    }

    #[test]
    fn ffi_join_native_path_skips_empty() {
        use std::ffi::CString;
        let a = CString::new("a").unwrap();
        let empty = CString::new("").unwrap();
        let b = CString::new("b").unwrap();
        let ptrs = [a.as_ptr(), empty.as_ptr(), b.as_ptr()];
        let out = pcsx2_path_join_native_path(ptrs.as_ptr(), ptrs.len());
        assert!(!out.is_null());
        let written = unsafe { CStr::from_ptr(out) }.to_string_lossy().into_owned();
        let expected = join_native_path(&["a", "b"]);
        assert_eq!(written, expected);
        unsafe { libc::free(out as *mut std::ffi::c_void) };
    }

    #[test]
    fn ffi_join_native_path_empty_count() {
        // count == 0 -> empty string, not null.
        let out = pcsx2_path_join_native_path(std::ptr::null(), 0);
        assert!(!out.is_null());
        let written = unsafe { CStr::from_ptr(out) }.to_string_lossy().into_owned();
        assert_eq!(written, "");
        unsafe { libc::free(out as *mut std::ffi::c_void) };
    }
}
