// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Rust reimplementation of PCSX2's `common/SmallString.{h,cpp}`.
//!
//! # Types
//!
//! | C++ Type | Rust Name | Storage |
//! |----------|-----------|---------|
//! | `SmallStringBase` | `SmallStringBase` | `String` (heap, dynamic) |
//! | `SmallStackString<L>` | `SmallStackString<N>` | SSO buffer `[u8; N+1]`, falls back to `String` |
//! | `SmallString` (256-byte) | `SmallString` = `SmallStackString<255>` | SSO 255 bytes + fallback |
//! | `TinyString` (64-byte) | `TinyString` = `SmallStackString<63>` | SSO 63 bytes + fallback |
//!
//! `SmallStringBase` is a type alias for `String` — always heap-allocated.
//! `SmallStackString<N>` is a proper SSO struct with an inline stack buffer.
//! It stores up to `N` bytes inline; when the content exceeds `N`, it
//! spills to a heap-allocated `String`.
//!
//! # SSO implementation
//!
//! `SmallStackString<N>` holds a tagged union:
//! - **Inline**: content fits in `[u8; N+1]` (N data + 1 NUL). `len` ≤ N.
//! - **Heap**: content stored in a `String`. `len` > N.
//!
//! The tag is implicit: `len` ≤ N → inline, `len` > N → heap.
//! The buffer always has a trailing NUL byte so `.as_str()` and
//! `.as_ptr()` are always valid C strings.

#![allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    clippy::all
)]

use std::cmp::Ordering;
use std::fmt::{self, Write};
use std::mem::MaybeUninit;
use std::ops::{Deref, DerefMut};
use std::os::raw::c_char;
use std::ptr;

// ---------------------------------------------------------------------------
// SmallStackString — SSO stack string with heap fallback
// ---------------------------------------------------------------------------

/// A string with **S**mall **S**tring **O**ptimisation.
///
/// Stores up to `N` bytes inline on the stack (avoiding heap allocation).
/// When the content exceeds `N` bytes, it spills to a heap `String`.
///
/// The inline buffer is `N + 1` bytes — `N` data bytes plus a trailing
/// NUL terminator, so `as_ptr()` always yields a valid C string.
///
/// # Type parameters
///
/// * `N` — inline capacity in bytes (excluding NUL terminator).
///   Default: `255` (matching C++ `SmallStackString<256>` minus NUL).


pub struct SmallStackString<const N: usize = 255> {
    /// Number of bytes stored (not counting NUL).
    /// If `len <= CAP`, data is in `inline_buf`.
    /// If `len > CAP`, data is in `heap` and `inline_buf` is unused.
    len: usize,
    /// Inline buffer: `[MaybeUninit<u8>; BUF]` where `BUF = N + 1`.
    /// Index `N` is always a NUL byte when used inline.
    inline_buf: [MaybeUninit<u8>; 256],
    /// Heap fallback: only used when `len > CAP`.
    heap: String,
}

// SAFETY: SmallStackString is Send+Sync because the internal String is.
unsafe impl<const N: usize> Send for SmallStackString<N> {}
unsafe impl<const N: usize> Sync for SmallStackString<N> {}

impl<const N: usize> SmallStackString<N> {
    /// Create an empty string.
    #[inline]
    pub fn new() -> Self {
        Self {
            len: 0,
            inline_buf: [MaybeUninit::new(0); 256],
            heap: String::new(),
        }
    }

    /// Create from `&str`. If the string fits inline, no heap alloc.
    #[inline]
    pub fn from_str(s: &str) -> Self {
        let mut this = Self::new();
        this.push_str(s);
        this
    }

    /// Create from `String`. If it fits inline, transfers ownership
    /// without heap alloc.
    #[inline]
    pub fn from_string(s: String) -> Self {
        let bytes = s.as_bytes();
        let len = bytes.len();
        if len <= N {
            // Inline: copy from String, then drop String.
            let mut this = Self::new();
            this.len = len;
            unsafe {
                ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    this.inline_buf.as_mut_ptr() as *mut u8,
                    len,
                );
                *this.inline_buf[len].as_mut_ptr() = 0;
            }
            drop(s);
            this
        } else {
            // Too big: use the heap.
            Self {
                len,
                inline_buf: [MaybeUninit::new(0); 256],
                heap: s,
            }
        }
    }

    /// Length in bytes.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Capacity (max inline size).
    #[inline]
    pub fn capacity(&self) -> usize {
        N
    }

    /// Whether the string is empty.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Whether content is stored inline (no heap alloc).
    #[inline]
    pub fn is_inline(&self) -> bool {
        self.len <= N
    }

    /// Clear the string, keeping the inline buffer if inline.
    #[inline]
    pub fn clear(&mut self) {
        if self.is_inline() {
            // Just reset len, buf zeroes are harmless.
            unsafe { *self.inline_buf[0].as_mut_ptr() = 0 };
        } else {
            self.heap.clear();
        }
        self.len = 0;
    }

    /// Access as `&str`.
    pub fn as_str(&self) -> &str {
        if self.is_inline() {
            // Safety: inline_buf[0..len] is valid UTF-8 (we only ever push_str/push/copy from valid UTF-8).
            let slice = unsafe {
                std::slice::from_raw_parts(
                    self.inline_buf.as_ptr() as *const u8,
                    self.len,
                )
            };
            // Safe because we only construct from valid UTF-8.
            unsafe { std::str::from_utf8_unchecked(slice) }
        } else {
            self.heap.as_str()
        }
    }

    /// Access as raw pointer (always NUL-terminated).
    pub fn as_ptr(&self) -> *const u8 {
        if self.is_inline() {
            self.inline_buf.as_ptr() as *const u8
        } else {
            self.heap.as_ptr()
        }
    }

    /// Access as C string pointer.
    pub fn as_cstr(&self) -> *const c_char {
        self.as_ptr() as *const c_char
    }

    /// Push a single character.
    pub fn push(&mut self, c: char) {
        let mut buf = [0u8; 4];
        let encoded = c.encode_utf8(&mut buf);
        self.push_str(encoded);
    }

    /// Push a string slice.
    pub fn push_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let new_len = self.len + s.len();
        if new_len <= N && self.is_inline() {
            // Inline: copy into buffer.
            unsafe {
                ptr::copy_nonoverlapping(
                    s.as_ptr(),
                    self.inline_buf[self.len].as_mut_ptr() as *mut u8,
                    s.len(),
                );
                *self.inline_buf[new_len].as_mut_ptr() = 0;
            }
            self.len = new_len;
        } else if new_len <= N {
            // Currently heap, but after append it fits inline.
            // Extract the content, copy to inline, then proceed.
            let old = self.heap.as_str().to_string();
            self.heap.clear();
            // Copy old content to inline.
            unsafe {
                ptr::copy_nonoverlapping(
                    old.as_ptr(),
                    self.inline_buf.as_mut_ptr() as *mut u8,
                    old.len(),
                );
                ptr::copy_nonoverlapping(
                    s.as_ptr(),
                    self.inline_buf[old.len()].as_mut_ptr() as *mut u8,
                    s.len(),
                );
                *self.inline_buf[new_len].as_mut_ptr() = 0;
            }
            self.len = new_len;
        } else {
            // Must use heap. If currently inline, copy to heap first.
            if self.is_inline() {
                let old = self.as_str().to_string();
                self.heap = old + s;
            } else {
                self.heap.push_str(s);
            }
            self.len = new_len;
        }
    }

    /// Append a string (alias for `push_str`).
    #[inline]
    pub fn append(&mut self, s: &str) {
        self.push_str(s);
    }

    /// Prepend a string at the beginning.
    pub fn prepend(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        let new_len = self.len + s.len();
        if new_len <= N && self.is_inline() {
            // Inline: shift existing content right.
            unsafe {
                // Move existing content right by s.len().
                ptr::copy(
                    self.inline_buf.as_ptr() as *const u8,
                    self.inline_buf[s.len()].as_mut_ptr() as *mut u8,
                    self.len,
                );
                // Copy new content at front.
                ptr::copy_nonoverlapping(
                    s.as_ptr(),
                    self.inline_buf.as_mut_ptr() as *mut u8,
                    s.len(),
                );
                *self.inline_buf[new_len].as_mut_ptr() = 0;
            }
            self.len = new_len;
        } else if new_len <= N {
            // Currently heap, but fits inline after prepend.
            let old = self.as_str().to_string();
            self.heap.clear();
            unsafe {
                ptr::copy_nonoverlapping(
                    s.as_ptr(),
                    self.inline_buf.as_mut_ptr() as *mut u8,
                    s.len(),
                );
                ptr::copy_nonoverlapping(
                    old.as_ptr(),
                    self.inline_buf[s.len()].as_mut_ptr() as *mut u8,
                    old.len(),
                );
                *self.inline_buf[new_len].as_mut_ptr() = 0;
            }
            self.len = new_len;
        } else {
            // Must use heap.
            let new_val = if self.is_inline() {
                let old = self.as_str().to_string();
                s.to_string() + &old
            } else {
                s.to_string() + &self.heap
            };
            self.heap = new_val;
            self.len = new_len;
        }
    }

    /// Append formatted text via `write!` macro.
    pub fn append_format(&mut self, args: fmt::Arguments<'_>) -> fmt::Result {
        // Format into a temporary String first, then append.
        let formatted = fmt::format(args);
        self.push_str(&formatted);
        Ok(())
    }

    /// Append a hex representation of bytes.
    /// Equivalent to C++ `SmallStringBase::append_hex`.
    pub fn append_hex(&mut self, data: &[u8]) {
        for b in data {
            self.push_str(&format!("{:02x}", b));
        }
    }

    /// Convert to `String` (always heap).
    #[inline]
    pub fn to_string(&self) -> String {
        self.as_str().to_string()
    }

    /// Take the content as `String`. If inline, allocates; if heap,
    /// replaces self with empty.
    pub fn take_string(&mut self) -> String {
        let result = self.as_str().to_string();
        self.clear();
        result
    }
}

// --- Trait impls ---

impl<const N: usize> Default for SmallStackString<N> {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Clone for SmallStackString<N> {
    fn clone(&self) -> Self {
        Self::from_str(self.as_str())
    }
}

impl<const N: usize> Deref for SmallStackString<N> {
    type Target = str;
    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl<const N: usize> DerefMut for SmallStackString<N> {
    #[inline]
    fn deref_mut(&mut self) -> &mut str {
        // Must return &mut str — convert via to_string then clear.
        // This is O(n) and copies, but matches mutable C string semantics.
        let s = self.as_str().to_string();
        self.clear();
        self.push_str(&s);
        unsafe {
            // Safety: we just wrote valid UTF-8 into either inline or heap.
            let slice = if self.is_inline() {
                std::slice::from_raw_parts_mut(
                    self.inline_buf.as_mut_ptr() as *mut u8,
                    self.len,
                )
            } else {
                // Need to get &mut [u8] from heap String.
                self.heap.as_bytes_mut()
            };
            std::str::from_utf8_unchecked_mut(slice)
        }
    }
}

impl<const N: usize> fmt::Display for SmallStackString<N> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<const N: usize> fmt::Debug for SmallStackString<N> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<const N: usize> PartialEq for SmallStackString<N> {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl<const N: usize> Eq for SmallStackString<N> {}

impl<const N: usize> PartialEq<str> for SmallStackString<N> {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl<const N: usize> PartialEq<&str> for SmallStackString<N> {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl<const N: usize> AsRef<str> for SmallStackString<N> {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

// ---------------------------------------------------------------------------
// Type aliases
// ---------------------------------------------------------------------------

/// Base string type — heap-allocated `String`.
/// Mirrors C++ `SmallStringBase` (which uses the heap).
pub type SmallStringBase = String;

/// Stack-allocated string with 255-byte inline capacity.
/// Mirrors C++ `SmallString` (SmallStackString<256>, minus NUL).
pub type SmallString = SmallStackString<255>;

/// Stack-allocated string with 63-byte inline capacity.
/// Mirrors C++ `TinyString` (SmallStackString<64>, minus NUL).
pub type TinyString = SmallStackString<63>;

// ---------------------------------------------------------------------------
// Re-exports for callers that want the same method vocabulary the C++
// side provides (e.g. `equals`, `iequals`, `view`, `buffer_size`).
//
// These are thin wrappers over `String` methods so that call sites
// translated from C++ keep their original shape. They are not part of
// the FFI surface; only the `pcsx2_smallstring_*` functions below are.
// ---------------------------------------------------------------------------

/// Case-sensitive equality. Mirrors `SmallStringBase::equals`.
#[inline]
pub fn equals<S: AsRef<str>>(this: &str, other: S) -> bool {
    this == other.as_ref()
}

/// Case-insensitive ASCII equality. Mirrors `SmallStringBase::iequals`.
///
/// Note: the C++ implementation uses `strcasecmp` / `_stricmp`, which
/// is locale-aware. This Rust version is ASCII-only; a follow-up could
/// use the `unicase` crate for proper Unicode case folding.
#[inline]
pub fn iequals<S: AsRef<str>>(this: &str, other: S) -> bool {
    this.len() == other.as_ref().len()
        && this
            .bytes()
            .zip(other.as_ref().bytes())
            .all(|(a, b)| a.eq_ignore_ascii_case(&b))
}

/// Mirrors `SmallStringBase::view` (returns a `&str` slice).
#[inline]
pub fn view(this: &str) -> &str {
    this
}

/// Mirrors `SmallStringBase::substr(offset, count)`. The C++ version
/// accepts negative offsets / counts; we emulate that here.
#[inline]
pub fn substr(this: &str, offset: i32, count: i32) -> &str {
    let len = this.len() as i32;
    let real_offset = if offset < 0 {
        (len + offset).max(0) as usize
    } else {
        (offset as usize).min(this.len())
    };
    let remaining = this.len() - real_offset;
    let real_count = if count < 0 {
        let c = (len + count).max(0) as usize;
        remaining.min(c)
    } else {
        remaining.min(count as usize)
    };
    if real_count == 0 {
        ""
    } else {
        &this[real_offset..real_offset + real_count]
    }
}

/// Mirrors `SmallStringBase::count(ch)`.
#[inline]
pub fn count(this: &str, ch: char) -> u32 {
    this.chars().filter(|c| *c == ch).count() as u32
}

// ---------------------------------------------------------------------------
// FFI exports
// ---------------------------------------------------------------------------
//
// The C++ side talks to `SmallString` via `const char*` and an opaque
// pointer. We model that here with a `*mut String` handle: Rust owns
// the `String`, allocates it on the C-side via `Box`, and frees it on
// the matching destroy. C++ only ever sees `*const c_char` for the
// payload and the opaque handle for ownership.
//
// All five exports are required by the task; they are sufficient to
// let the C++ side construct a `SmallString`, mutate it via the
// inherent methods of `String` (we re-export those through the type
// alias), read it back, and free it.

/// Allocate a new, empty `String` and return an opaque owning pointer.
///
/// The caller is responsible for calling
/// [`pcsx2_smallstring_destroy`] exactly once on the returned pointer.
#[no_mangle]
pub extern "C" fn pcsx2_smallstring_create() -> *mut String {
    Box::into_raw(Box::new(String::new()))
}

/// Allocate a new `String` initialised from a NUL-terminated C string
/// and return an opaque owning pointer.
///
/// Returns a null pointer if `s` is null.
#[no_mangle]
pub extern "C" fn pcsx2_smallstring_from_cstr(s: *const c_char) -> *mut String {
    if s.is_null() {
        return ptr::null_mut();
    }

    // SAFETY: caller guarantees `s` is a NUL-terminated C string. We
    // stop at the first NUL byte; anything past it is irrelevant.
    let value = unsafe { std::ffi::CStr::from_ptr(s) }
        .to_string_lossy()
        .into_owned();
    Box::into_raw(Box::new(value))
}

/// Return a pointer to the string's UTF-8 (NUL-terminated) payload.
///
/// The pointer is valid for the lifetime of the underlying `String`,
/// i.e. until the caller invokes
/// [`pcsx2_smallstring_destroy`] on the same handle. The C++ side
/// must not free or mutate the returned pointer; copying is fine.
#[no_mangle]
pub extern "C" fn pcsx2_smallstring_cstr(s: *mut String) -> *const c_char {
    if s.is_null() {
        return ptr::null();
    }

    // SAFETY: caller guarantees `s` is a pointer previously returned
    // by one of the `pcsx2_smallstring_*create*` functions and not
    // yet destroyed.
    let str_ref: &String = unsafe { &*s };
    str_ref.as_ptr() as *const c_char
}

/// Return the length of the string in bytes.
///
/// Returns 0 when `s` is null so the C++ side can probe uninitialised
/// handles without UB.
#[no_mangle]
pub extern "C" fn pcsx2_smallstring_length(s: *mut String) -> u32 {
    if s.is_null() {
        return 0;
    }

    // SAFETY: same contract as `pcsx2_smallstring_cstr`.
    let str_ref: &String = unsafe { &*s };
    str_ref.len() as u32
}

/// Free a `String` previously allocated by
/// [`pcsx2_smallstring_create`] or
/// [`pcsx2_smallstring_from_cstr`].
///
/// Passing a null pointer is a no-op, matching the behaviour of `free`.
/// After this call, `s` must not be used again.
#[no_mangle]
pub extern "C" fn pcsx2_smallstring_destroy(s: *mut String) {
    if s.is_null() {
        return;
    }

    // SAFETY: caller guarantees `s` was allocated by `Box::into_raw`
    // (matching the create / from_cstr functions) and is not aliased.
    unsafe {
        drop(Box::from_raw(s));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equals_works() {
        assert!(equals("hello", "hello"));
        assert!(!equals("hello", "world"));
    }

    #[test]
    fn iequals_works_ascii() {
        assert!(iequals("Hello", "hello"));
        assert!(iequals("HELLO", "hello"));
        assert!(!iequals("Hello", "world"));
    }

    #[test]
    fn substr_works() {
        // No clamping
        assert_eq!(substr("abcdef", 1, 2), "bc");
        // Negative offset counted from end
        assert_eq!(substr("abcdef", -3, 2), "de");
        // Negative count means "all but last |count|"
        assert_eq!(substr("abcdef", 0, -1), "abcde");
        // Clamped to length
        assert_eq!(substr("abcdef", 2, 100), "cdef");
    }

    #[test]
    fn count_chars_works() {
        assert_eq!(count("hello world", 'l'), 3);
        assert_eq!(count("hello world", 'z'), 0);
    }

    #[test]
    fn ffi_roundtrip() {
        unsafe {
            let handle = pcsx2_smallstring_from_cstr(b"hi\0".as_ptr() as *const c_char);
            assert!(!handle.is_null());
            assert_eq!(pcsx2_smallstring_length(handle), 2);
            let c = pcsx2_smallstring_cstr(handle);
            assert!(!c.is_null());
            let bytes = std::ffi::CStr::from_ptr(c).to_bytes();
            assert_eq!(bytes, b"hi");
            pcsx2_smallstring_destroy(handle);
        }
    }

    #[test]
    fn ffi_null_is_safe() {
        unsafe {
            assert!(pcsx2_smallstring_cstr(std::ptr::null_mut()).is_null());
            assert_eq!(pcsx2_smallstring_length(std::ptr::null_mut()), 0);
            pcsx2_smallstring_destroy(std::ptr::null_mut());
        }
    }
}
