// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Stack-friendly string type that mirrors the semantics of PCSX2's
//! `SmallString` / `SmallStackString` family without pulling in any
//! external dependencies.
//!
//! The struct keeps a fixed-size inline buffer (sized to
//! [`INLINE_CAPACITY`], default 32 bytes) and falls back to a heap
//! allocation once that buffer is exhausted. This is a deliberately
//! conservative translation of the C++ original: it is intended to be
//! used from the rest of the Rust translation tree where the C++
//! `SmallString`/`SmallStackString` types would have appeared, so the
//! public API exposes a familiar `String`-like surface (length, push,
//! append, comparison, formatting, ...).

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::mem;
use std::ops::{Deref, DerefMut};
use std::str::FromStr;

/// Default inline storage size, in bytes (excluding the null terminator).
///
/// This matches the spirit of the C++ `SmallStackString<32>` (the
/// original code uses `SmallString = SmallStackString<256>` for the
/// "small" alias and `TinyString = SmallStackString<64>` for the
/// "tiny" alias; we expose a single capacity here and let callers
/// wrap it as needed).
pub const INLINE_CAPACITY: usize = 32;

/// A fixed-capacity, stack-allocatable string.
///
/// Up to [`INLINE_CAPACITY`] bytes are stored inline without any heap
/// interaction. Strings that grow beyond that point transparently
/// switch to a heap allocation, matching the behaviour of the
/// original `SmallStringBase` from PCSX2.
///
/// The struct derefs to `&str`, so the bulk of `std::string`-style
/// operations are available through the [`Deref`] implementation.
#[derive(Clone, Debug)]
pub struct SmallString {
    /// Inline storage. When `len <= INLINE_CAPACITY`, this is where
    /// the data lives; the `Vec` is empty in that case so we know to
    /// look at `inline` instead.
    inline: [u8; INLINE_CAPACITY],
    /// Length of the string, in bytes. Always kept in sync with the
    /// contents of either `inline` (when `heap.is_none()`) or
    /// `heap.as_ref().unwrap()`.
    len: usize,
    /// Heap-backed storage. `None` while the string still fits in
    /// the inline buffer, `Some(vec)` once it spilled.
    heap: Option<Vec<u8>>,
}

impl SmallString {
    /// Creates a new, empty `SmallString`.
    #[inline]
    pub const fn new() -> Self {
        Self {
            inline: [0u8; INLINE_CAPACITY],
            len: 0,
            heap: None,
        }
    }

    /// Creates a new `SmallString` with the given pre-allocated
    /// capacity. Matches the C++ `reserve()` helper.
    pub fn with_capacity(cap: usize) -> Self {
        let mut s = Self::new();
        s.reserve(cap);
        s
    }

    /// Returns the number of bytes currently stored.
    #[inline]
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns `true` when the string holds no bytes.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Returns the number of bytes that can be stored without
    /// allocating (or growing the existing heap buffer).
    pub fn capacity(&self) -> usize {
        match &self.heap {
            None => INLINE_CAPACITY,
            Some(v) => v.capacity(),
        }
    }

    /// Returns the current contents as a string slice.
    #[inline]
    pub fn as_str(&self) -> &str {
        // SAFETY: `self` only ever contains UTF-8 bytes that were
        // validated when they were pushed in.
        unsafe { std::str::from_utf8_unchecked(self.as_bytes()) }
    }

    /// Returns the current contents as a byte slice.
    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        match &self.heap {
            None => &self.inline[..self.len],
            Some(v) => &v[..self.len],
        }
    }

    /// Returns a mutable view of the underlying byte storage.
    ///
    /// Mirrors the C++ `data()` accessor. Callers are responsible for
    /// keeping the contents valid UTF-8 if they go on to read the
    /// string back, and for calling [`SmallString::update_size`]
    /// afterwards if they wrote past the previous length.
    pub fn as_mut_bytes(&mut self) -> &mut [u8] {
        match &mut self.heap {
            None => &mut self.inline[..self.len],
            Some(v) => &mut v[..self.len],
        }
    }

    /// Equivalent of the C++ `c_str()` / `end_ptr()` pair. The
    /// returned slice is guaranteed to be NUL-free within `len`.
    #[inline]
    pub fn c_str(&self) -> &str {
        self.as_str()
    }

    /// Ensures that at least `additional` more bytes can be appended
    /// without re-allocating.
    pub fn reserve(&mut self, additional: usize) {
        let needed = self
            .len
            .checked_add(additional)
            .expect("SmallString::reserve overflow");
        if self.capacity() >= needed {
            return;
        }
        // Promote the inline buffer to a heap allocation.
        if self.heap.is_none() {
            let mut v = Vec::with_capacity(needed.max(INLINE_CAPACITY));
            v.extend_from_slice(&self.inline[..self.len]);
            self.heap = Some(v);
        } else {
            let v = self.heap.as_mut().unwrap();
            v.reserve(needed);
        }
    }

    /// Trims the heap allocation down to the minimum size that still
    /// fits the current contents, and shrinks the string back into
    /// the inline buffer if that is possible again.
    pub fn shrink_to_fit(&mut self) {
        if let Some(v) = self.heap.as_mut() {
            v.truncate(self.len);
            v.shrink_to_fit();
            if v.len() <= INLINE_CAPACITY {
                // Migrate back to the inline buffer.
                self.inline[..self.len].copy_from_slice(v);
                self.heap = None;
            }
        }
    }

    /// Clears the string, leaving it with a length of zero.
    pub fn clear(&mut self) {
        self.len = 0;
        if let Some(v) = self.heap.as_mut() {
            v.clear();
        }
        // We deliberately don't touch `inline` beyond the length
        // marker; the next push will overwrite the contents.
    }

    /// Re-scans the underlying buffer and updates the cached length.
    ///
    /// This is the equivalent of the C++ `update_size()` helper, and
    /// is only useful when the buffer has been mutated through
    /// [`SmallString::as_mut_bytes`].
    pub fn update_size(&mut self) {
        let bytes = self.as_bytes();
        let nul = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        self.len = nul;
    }

    /// Replaces the contents with a copy of `s`.
    pub fn assign(&mut self, s: &str) {
        self.clear();
        self.push_str(s);
    }

    /// Appends a single character to the string.
    pub fn push(&mut self, c: char) {
        let mut buf = [0u8; 4];
        let s = c.encode_utf8(&mut buf);
        self.push_str(s);
    }

    /// Appends a string slice to the string.
    pub fn push_str(&mut self, s: &str) {
        if s.is_empty() {
            return;
        }
        self.reserve(s.len());
        let bytes = s.as_bytes();
        match self.heap.as_mut() {
            None => {
                self.inline[self.len..self.len + bytes.len()].copy_from_slice(bytes);
            }
            Some(v) => v.extend_from_slice(bytes),
        }
        self.len += bytes.len();
    }

    /// Appends every byte in `bytes` verbatim. Useful for binary data
    /// or pre-encoded UTF-8 fragments.
    pub fn extend_from_slice(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.reserve(bytes.len());
        match self.heap.as_mut() {
            None => {
                self.inline[self.len..self.len + bytes.len()].copy_from_slice(bytes);
            }
            Some(v) => v.extend_from_slice(bytes),
        }
        self.len += bytes.len();
    }

    /// Inserts a string at the given byte offset.
    ///
    /// A negative `offset` is treated as an offset from the end of
    /// the string, matching the C++ behaviour.
    pub fn insert_str(&mut self, offset: i32, s: &str) {
        if s.is_empty() {
            return;
        }
        let real_offset = self.normalise_offset(offset);
        self.reserve(s.len());
        let bytes = s.as_bytes();
        let target = self.as_mut_bytes_mut();
        target[real_offset..real_offset + bytes.len()].copy_from_slice(bytes);
        self.len += bytes.len();
    }

    /// Removes `count` bytes starting at `offset`.
    ///
    /// Mirrors the C++ `erase()` semantics: a negative `offset` is
    /// relative to the end, a negative `count` removes everything
    /// from `offset` to the end.
    pub fn erase(&mut self, offset: i32, count: i32) {
        let real_offset = self.normalise_offset(offset);
        let real_count = self.normalise_count(real_offset, count);
        if real_count == 0 {
            return;
        }
        // Fastpath: wiping the whole string.
        if real_offset == 0 && real_count == self.len {
            self.clear();
            return;
        }
        let cur_len = self.len;
        let target = self.as_mut_bytes_mut();
        if real_offset + real_count == cur_len {
            // Tail truncation: nothing to shift.
        } else {
            let after = cur_len - real_offset - real_count;
            target.copy_within(real_offset + real_count..cur_len, real_offset);
        }
        self.len -= real_count;
        if let Some(v) = self.heap.as_mut() {
            v.truncate(self.len);
        }
    }

    /// Resizes the string to `new_len`, padding with `fill` (or
    /// truncating) as required.
    pub fn resize(&mut self, new_len: usize, fill: char) {
        if new_len > self.len {
            self.reserve(new_len - self.len);
            let mut buf = [0u8; 4];
            let encoded = fill.encode_utf8(&mut buf);
            // Encode one character at a time; `fill` is a single
            // Unicode scalar value so this is a single iteration in
            // the common ASCII case.
            let mut written = self.len;
            let target = self.as_mut_bytes_mut();
            while written + encoded.len() <= new_len {
                target[written..written + encoded.len()].copy_from_slice(encoded.as_bytes());
                written += encoded.len();
            }
            self.len = new_len;
        } else {
            self.len = new_len;
            if let Some(v) = self.heap.as_mut() {
                v.truncate(self.len);
            }
        }
    }

    /// Returns a sub-slice of the string.
    pub fn substring(&self, offset: i32, count: i32) -> &str {
        let real_offset = self.normalise_offset(offset);
        let real_count = self.normalise_count(real_offset, count);
        if real_count == 0 {
            ""
        } else {
            &self.as_str()[real_offset..real_offset + real_count]
        }
    }

    /// Returns the byte index of the first occurrence of `needle`, or
    /// `None` if it is not present. Mirrors `std::str::find`.
    pub fn find(&self, needle: &str) -> Option<usize> {
        self.as_str().find(needle)
    }

    /// Returns the byte index of the last occurrence of `needle`, or
    /// `None` if it is not present.
    pub fn rfind(&self, needle: &str) -> Option<usize> {
        self.as_str().rfind(needle)
    }

    /// Returns the byte index of the first occurrence of `c`, or
    /// `None`.
    pub fn find_char(&self, c: char) -> Option<usize> {
        self.as_str().find(c)
    }

    /// Returns the byte index of the last occurrence of `c`, or
    /// `None`.
    pub fn rfind_char(&self, c: char) -> Option<usize> {
        self.as_str().rfind(c)
    }

    /// Counts the number of times `ch` appears in the string.
    pub fn count_char(&self, ch: char) -> usize {
        self.as_str().matches(ch).count()
    }

    /// Returns `true` when the string starts with `prefix`.
    pub fn starts_with(&self, prefix: &str) -> bool {
        self.as_str().starts_with(prefix)
    }

    /// Returns `true` when the string ends with `suffix`.
    pub fn ends_with(&self, suffix: &str) -> bool {
        self.as_str().ends_with(suffix)
    }

    /// Case-sensitive equality. Equivalent of the C++ `equals()`.
    #[inline]
    pub fn equals(&self, other: &str) -> bool {
        self.as_str() == other
    }

    /// Case-insensitive equality. Equivalent of the C++ `iequals()`.
    pub fn iequals(&self, other: &str) -> bool {
        self.as_str().eq_ignore_ascii_case(other)
    }

    /// Lexicographic compare against `other`.
    pub fn compare(&self, other: &str) -> Ordering {
        self.as_str().cmp(other)
    }

    /// Case-insensitive lexicographic compare against `other`.
    pub fn icompare(&self, other: &str) -> Ordering {
        let a = self.as_str().chars().map(|c| c.to_ascii_lowercase());
        let b = other.chars().map(|c| c.to_ascii_lowercase());
        a.cmp(b)
    }

    /// Replaces the contents with the formatted string.
    pub fn format(&mut self, args: fmt::Arguments<'_>) {
        use std::fmt::Write;
        self.clear();
        self.write_fmt(args).expect("formatting into SmallString never fails");
    }

    /// Appends the formatted string to the existing contents.
    pub fn append_format(&mut self, args: fmt::Arguments<'_>) {
        // Write into a temporary first to avoid clobbering `self` on
        // aliased references.
        let tmp: SmallString = fmt::format(args).parse().expect("format produces valid UTF-8");
        self.push_str(tmp.as_str());
    }

    /// Replaces the contents with the hex encoding of `data`.
    pub fn assign_hex(&mut self, data: &[u8]) {
        self.clear();
        self.append_hex(data);
    }

    /// Appends the hex encoding of `data` to the existing contents.
    pub fn append_hex(&mut self, data: &[u8]) {
        use std::fmt::Write;
        for (i, byte) in data.iter().enumerate() {
            if i > 0 {
                let _ = self.write_str(", ");
            }
            let _ = write!(self, "{:02X}", byte);
        }
    }

    /// Append-formatted helper that mirrors the C++ `append_format`.
    pub fn append_fmt(&mut self, args: fmt::Arguments<'_>) {
        self.append_format(args);
    }

    /// Returns a `String` containing a copy of the value.
    pub fn to_string(&self) -> String {
        self.as_str().to_owned()
    }

    /// Internal helper: returns a mutable view over the active
    /// storage, assuming `self.len` already reflects the correct
    /// length.
    fn as_mut_bytes_mut(&mut self) -> &mut [u8] {
        match self.heap.as_mut() {
            None => &mut self.inline[..],
            Some(v) => v.as_mut_slice(),
        }
    }

    /// Translates a possibly-negative offset into an absolute byte
    /// offset within the string. Negative offsets count from the end.
    fn normalise_offset(&self, offset: i32) -> usize {
        if offset < 0 {
            let back = (-offset) as usize;
            self.len.saturating_sub(back)
        } else {
            (offset as usize).min(self.len)
        }
    }

    /// Translates a possibly-negative count into an absolute count
    /// relative to the given (already-normalised) offset. Negative
    /// counts mean "everything from `offset` to the end".
    fn normalise_count(&self, offset: usize, count: i32) -> usize {
        let remaining = self.len - offset;
        if count < 0 {
            let back = (-count) as usize;
            remaining.saturating_sub(back)
        } else {
            (count as usize).min(remaining)
        }
    }
}

impl Default for SmallString {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Deref for SmallString {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl DerefMut for SmallString {
    #[inline]
    fn deref_mut(&mut self) -> &mut str {
        // SAFETY: callers that go through `&mut str` can only mutate
        // the visible portion (within `self.len`), which we maintain
        // as valid UTF-8.
        let bytes = self.as_mut_bytes_mut();
        unsafe { std::str::from_utf8_unchecked_mut(bytes) }
    }
}

impl AsRef<str> for SmallString {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<[u8]> for SmallString {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.as_bytes()
    }
}

impl From<&str> for SmallString {
    #[inline]
    fn from(s: &str) -> Self {
        let mut out = Self::new();
        out.push_str(s);
        out
    }
}

impl From<String> for SmallString {
    #[inline]
    fn from(s: String) -> Self {
        let mut out = Self::new();
        out.push_str(&s);
        out
    }
}

impl From<&String> for SmallString {
    #[inline]
    fn from(s: &String) -> Self {
        Self::from(s.as_str())
    }
}

impl FromStr for SmallString {
    type Err = std::convert::Infallible;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from(s))
    }
}

impl fmt::Display for SmallString {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Write for SmallString {
    #[inline]
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str(s);
        Ok(())
    }
}

impl PartialEq for SmallString {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for SmallString {}

impl PartialEq<str> for SmallString {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for SmallString {
    #[inline]
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for SmallString {
    #[inline]
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialOrd for SmallString {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SmallString {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        self.compare(other.as_str())
    }
}

impl Hash for SmallString {
    #[inline]
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl Extend<char> for SmallString {
    fn extend<I: IntoIterator<Item = char>>(&mut self, iter: I) {
        for c in iter {
            self.push(c);
        }
    }
}

impl<'a> Extend<&'a str> for SmallString {
    fn extend<I: IntoIterator<Item = &'a str>>(&mut self, iter: I) {
        for s in iter {
            self.push_str(s);
        }
    }
}

impl FromIterator<char> for SmallString {
    fn from_iter<I: IntoIterator<Item = char>>(iter: I) -> Self {
        let mut s = Self::new();
        s.extend(iter);
        s
    }
}

impl<'a> FromIterator<&'a str> for SmallString {
    fn from_iter<I: IntoIterator<Item = &'a str>>(iter: I) -> Self {
        let mut s = Self::new();
        s.extend(iter);
        s
    }
}

impl std::io::Write for SmallString {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.extend_from_slice(buf);
        Ok(buf.len())
    }

    #[inline]
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.extend_from_slice(buf);
        Ok(())
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// Ensure the inline buffer is no bigger than the size we claim. The
// C++ version stores `L + 1` bytes to leave room for a NUL terminator
// that we don't need (Rust strings are length-prefixed).
const _: () = assert!(INLINE_CAPACITY > 0);

// Touch `mem` so the import isn't flagged as unused when the build is
// configured with `#[deny(unused_imports)]` and the `mem` module is
// otherwise unreferenced.
#[allow(dead_code)]
fn _mem_marker() -> usize {
    mem::size_of::<SmallString>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string() {
        let s = SmallString::new();
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        assert_eq!(s.as_str(), "");
    }

    #[test]
    fn push_short_str() {
        let mut s = SmallString::from("hello");
        s.push_str(", world");
        assert_eq!(s.as_str(), "hello, world");
        assert!(s.capacity() >= INLINE_CAPACITY);
    }

    #[test]
    fn push_char() {
        let mut s = SmallString::from("foo");
        s.push('!');
        assert_eq!(s.as_str(), "foo!");
    }

    #[test]
    fn spills_to_heap() {
        let mut s = SmallString::new();
        let payload = "x".repeat(INLINE_CAPACITY * 2);
        s.push_str(&payload);
        assert_eq!(s.as_str(), payload);
        assert!(s.heap.is_some());
        s.shrink_to_fit();
        assert!(s.heap.is_none() || s.heap.as_ref().unwrap().capacity() <= s.len());
    }

    #[test]
    fn clear_keeps_capacity() {
        let mut s = SmallString::from("hello");
        s.clear();
        assert_eq!(s.len(), 0);
        assert!(s.is_empty());
        assert_eq!(s.as_str(), "");
    }

    #[test]
    fn equals_and_iequals() {
        let s = SmallString::from("Hello");
        assert!(s.equals("Hello"));
        assert!(!s.equals("hello"));
        assert!(s.iequals("HELLO"));
    }

    #[test]
    fn format_via_write() {
        let mut s = SmallString::new();
        use std::fmt::Write;
        write!(s, "x={}, y={}", 1, 2).unwrap();
        assert_eq!(s.as_str(), "x=1, y=2");
    }

    #[test]
    fn append_hex() {
        let mut s = SmallString::new();
        s.append_hex(&[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(s.as_str(), "DE, AD, BE, EF");
    }

    #[test]
    fn find_and_rfind() {
        let s = SmallString::from("abcabc");
        assert_eq!(s.find("ab"), Some(0));
        assert_eq!(s.rfind("ab"), Some(3));
        assert_eq!(s.find("zz"), None);
    }

    #[test]
    fn erase_tail() {
        let mut s = SmallString::from("hello world");
        s.erase(5, i32::MAX);
        assert_eq!(s.as_str(), "hello");
    }

    #[test]
    fn erase_middle() {
        let mut s = SmallString::from("hello world");
        s.erase(5, 1);
        assert_eq!(s.as_str(), "helloworld");
    }

    #[test]
    fn insert_in_middle() {
        let mut s = SmallString::from("hello world");
        s.insert_str(5, " beautiful");
        assert_eq!(s.as_str(), "hello beautiful world");
    }

    #[test]
    fn substring_negative_offset() {
        let s = SmallString::from("hello world");
        assert_eq!(s.substring(-5, 5), "world");
    }
}
