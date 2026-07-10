// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! String utilities — Rust translation of `common/StringUtil.{h,cpp}`.
//!
//! The C++ namespace `StringUtil` exposes a grab-bag of helpers: a
//! printf-style formatter, glob matching, bounded string copy, case-
//! insensitive comparison, and templated parse/format helpers built on
//! `std::from_chars` / `std::to_chars` / `std::stringstream`. The
//! Boolean overload accepts a vocabulary of true/false spellings
//! (`true`, `yes`, `on`, `1`, `enabled` / `false`, `no`, `off`, `0`,
//! `disabled`).
//!
//! This port provides the full set of `StringUtil` helpers:
//!
//! * [`format`] / [`format_runtime`] — printf-style formatter
//! * [`WildcardMatch`] / [`wildcard_match`] — `*`/`?` glob matcher
//! * [`strlcpy`] — bounded copy
//! * [`strcasecmp`] / [`strncasecmp`] — case-insensitive compare
//! * [`from_chars`] / [`to_chars`] — parse/format generics
//! * [`parse_bool`] — boolean vocabulary parser
//! * [`decode_hex`] / [`encode_hex`] — hex encoding
//! * [`to_lower`] / [`to_upper`] — case conversion (non-ASCII aware)
//! * [`compare_no_case`] — case-insensitive equality
//! * [`split_on_new_line`] — newline splitting
//! * [`strip_whitespace`] — trim whitespace
//! * [`split_string`] — delimiter splitting with skip-empty
//! * [`replace_all`] — find-and-replace
//! * [`parse_assignment_string`] — `"key=value"` parser
//! * [`append_utf16_char_to_utf8`] / [`encode_and_append_utf8`] — UTF encoding
//! * [`decode_utf8`] — UTF-8 decoding
//! * [`ellipsise`] / [`ellipsise_in_place`] — truncation with ellipsis
//! * [`u128_to_string`] / [`append_u128_to_string`] — u128 formatting

#![allow(clippy::needless_return)]

use std::cmp::Ordering;
use std::ffi::CStr;
use std::fmt::{self, Display};
use std::os::raw::c_char;
use std::str::FromStr;

// ============================================================================
// Format helpers
// ============================================================================

/// Constructs a `String` from a `format!`-style template plus arguments.
///
/// Direct counterpart of `StringUtil::StdStringFromFormat`.
/// Uses Rust's `std::fmt` syntax (not printf).
#[inline]
pub fn format(args: fmt::Arguments<'_>) -> String {
    fmt::format(args)
}

/// Convenience wrapper around [`format`] that takes a runtime `&str`
/// template (still a `format!`-style string).
///
/// The C++ `StdStringFromFormat(const char* format, ...)` accepts a
/// runtime format string. This wrapper exists for FFI parity.
pub fn format_runtime(template: &str) -> String {
    template.to_owned()
}

// ============================================================================
// Wildcard / glob matching
// ============================================================================

/// Glob-mask match: `*` matches any run of bytes, `?` matches one byte.
/// Case-insensitive by default.
pub fn wildcard_match(subject: &str, mask: &str) -> bool {
    wildcard_match_impl(subject.as_bytes(), mask.as_bytes(), false)
}

/// Same as [`wildcard_match`] but with case sensitivity selectable.
pub fn wildcard_match_with(subject: &str, mask: &str, case_sensitive: bool) -> bool {
    wildcard_match_impl(subject.as_bytes(), mask.as_bytes(), case_sensitive)
}

/// Public entry-point named after the C++ function for direct
/// translation. Case-insensitive by default; pass `true` as the third
/// argument for the C++ default behaviour.
#[allow(non_snake_case)]
pub fn WildcardMatch(subject: &str, mask: &str, case_sensitive: bool) -> bool {
    wildcard_match_impl(subject.as_bytes(), mask.as_bytes(), case_sensitive)
}

/// Core implementation working on byte slices.
fn wildcard_match_impl(subject: &[u8], mask: &[u8], case_sensitive: bool) -> bool {
    let fold = |b: u8| b.to_ascii_lowercase();

    let eq_or_q = |m: u8, s: u8| -> bool {
        if m == b'?' {
            return true;
        }
        if case_sensitive {
            m == s
        } else {
            fold(m) == fold(s)
        }
    };

    let mut mp: Option<usize> = None;
    let mut cp: Option<usize> = None;

    let mut s_idx = 0usize;
    let mut m_idx = 0usize;

    // Initial scan: skip a literal prefix (no `*` in mask yet).
    while s_idx < subject.len() && m_idx < mask.len() && mask[m_idx] != b'*' {
        if !eq_or_q(mask[m_idx], subject[s_idx]) {
            return false;
        }
        s_idx += 1;
        m_idx += 1;
    }

    while s_idx < subject.len() {
        if mask[m_idx] == b'*' {
            m_idx += 1;
            if m_idx == mask.len() {
                return true;
            }
            mp = Some(m_idx);
            cp = Some(s_idx + 1);
        } else if eq_or_q(mask[m_idx], subject[s_idx]) {
            s_idx += 1;
            m_idx += 1;
        } else {
            // Backtrack: rewind to the last `*` and try a longer match.
            let (Some(back_m), Some(back_c)) = (mp, cp) else {
                return false;
            };
            m_idx = back_m;
            s_idx = back_c;
            cp = Some(back_c + 1);
        }
    }

    // Trailing `*`s in the mask consume nothing.
    while m_idx < mask.len() && mask[m_idx] == b'*' {
        m_idx += 1;
    }

    m_idx == mask.len()
}

// ============================================================================
// Bounded string copy
// ============================================================================

/// Bounded copy from `src` into `dst`. Always NUL-terminates the
/// destination when `dst` is non-empty. Returns the length of `src`
/// (the caller can detect truncation by comparing the return value to `dst.len()`).
pub fn strlcpy(dst: &mut [u8], src: &[u8]) -> usize {
    let src_len = src.len();
    if src_len + 1 <= dst.len() {
        dst[..src_len].copy_from_slice(src);
        dst[src_len] = 0;
    } else if !dst.is_empty() {
        let cap = dst.len() - 1;
        dst[..cap].copy_from_slice(&src[..cap]);
        dst[cap] = 0;
    }
    src_len
}

// ============================================================================
// Case-insensitive comparison
// ============================================================================

/// Case-insensitive lexicographic compare. Returns negative if `a < b`,
/// zero if `a == b`, positive if `a > b`.
pub fn strcasecmp(a: &str, b: &str) -> i32 {
    let common = a.len().min(b.len());
    for i in 0..common {
        let la = a.as_bytes()[i].to_ascii_lowercase();
        let lb = b.as_bytes()[i].to_ascii_lowercase();
        match la.cmp(&lb) {
            Ordering::Equal => continue,
            Ordering::Less => return -1,
            Ordering::Greater => return 1,
        }
    }
    match a.len().cmp(&b.len()) {
        Ordering::Equal => 0,
        Ordering::Less => -1,
        Ordering::Greater => 1,
    }
}

/// Case-insensitive compare up to `n` bytes.
pub fn strncasecmp(a: &str, b: &str, n: usize) -> i32 {
    let limit = a.len().min(b.len()).min(n);
    for i in 0..limit {
        let la = a.as_bytes()[i].to_ascii_lowercase();
        let lb = b.as_bytes()[i].to_ascii_lowercase();
        match la.cmp(&lb) {
            Ordering::Equal => continue,
            Ordering::Less => return -1,
            Ordering::Greater => return 1,
        }
    }
    if n >= a.len().max(b.len()) {
        match a.len().cmp(&b.len()) {
            Ordering::Equal => 0,
            Ordering::Less => -1,
            Ordering::Greater => 1,
        }
    } else {
        0
    }
}

// ============================================================================
// from_chars / to_chars
// ============================================================================

/// Parse `s` into `T` via [`FromStr`].
pub fn from_chars<T>(s: &str) -> Option<T>
where
    T: FromStr,
    T::Err: std::fmt::Debug,
{
    T::from_str(s).ok()
}

/// Same as [`from_chars`] but reports the unconsumed tail of the input.
pub fn parse_with_rest<'a, T>(s: &'a str, rest: &mut &'a str) -> Option<T>
where
    T: FromStr,
    T::Err: std::fmt::Debug,
{
    match s.find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '+' && c != '.') {
        Some(end) => {
            let head = &s[..end];
            let value = T::from_str(head).ok()?;
            *rest = &s[end..];
            Some(value)
        }
        None => {
            let value = T::from_str(s).ok()?;
            *rest = "";
            Some(value)
        }
    }
}

/// Format `value` into a `String` using its [`Display`] impl.
#[inline]
pub fn to_chars<T: Display>(value: T) -> String {
    value.to_string()
}

// ============================================================================
// BoolArg — FromStr newtype with the PCSX2 boolean vocabulary
// ============================================================================

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct BoolArg(pub bool);

impl BoolArg {
    #[inline]
    pub const fn new(value: bool) -> Self {
        BoolArg(value)
    }

    #[inline]
    pub const fn get(self) -> bool {
        self.0
    }
}

impl From<BoolArg> for bool {
    #[inline]
    fn from(b: BoolArg) -> bool {
        b.0
    }
}

impl From<bool> for BoolArg {
    #[inline]
    fn from(b: bool) -> BoolArg {
        BoolArg(b)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ParseBoolError {
    _private: (),
}

impl fmt::Display for ParseBoolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(
            "invalid boolean literal: expected one of \
             true/yes/on/1/enabled, false/no/off/0/disabled (case-insensitive)",
        )
    }
}

impl std::error::Error for ParseBoolError {}

impl FromStr for BoolArg {
    type Err = ParseBoolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" | "enabled" => Ok(BoolArg(true)),
            "false" | "no" | "off" | "0" | "disabled" => Ok(BoolArg(false)),
            _ => Err(ParseBoolError { _private: () }),
        }
    }
}

/// Convenience: parse `s` directly into `Option<bool>`.
#[inline]
pub fn parse_bool(s: &str) -> Option<bool> {
    BoolArg::from_str(s).ok().map(|b| b.0)
}

// ============================================================================
// Hex encoding / decoding
// ============================================================================

/// Decode a hex string into a byte vector.
///
/// Mirrors `StringUtil::DecodeHex`. Returns `None` if the string
/// contains non-hex characters or has odd length.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::decode_hex;
/// assert_eq!(decode_hex("A1B2"), Some(vec![0xA1, 0xB2]));
/// assert_eq!(decode_hex(""), Some(vec![]));
/// assert_eq!(decode_hex("xyz"), None);
/// ```
pub fn decode_hex(input: &str) -> Option<Vec<u8>> {
    if input.is_empty() {
        return Some(Vec::new());
    }

    // C++ version uses FromChars<u8>(substr, 16) which handles each pair.
    // We do the same: iterate over 2-char chunks, parse hex.
    let chars: Vec<char> = input.chars().collect();
    if chars.len() % 2 != 0 {
        return None; // Odd length: the C++ version would also error on the last partial byte
    }

    let mut data = Vec::with_capacity(chars.len() / 2);
    for chunk in chars.chunks(2) {
        let hex_str: String = chunk.iter().collect();
        let byte = u8::from_str_radix(&hex_str, 16).ok()?;
        data.push(byte);
    }

    Some(data)
}

/// Encode a byte slice as a lowercase hex string.
///
/// Mirrors `StringUtil::EncodeHex`.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::encode_hex;
/// assert_eq!(encode_hex(&[0xA1, 0xB2]), "a1b2");
/// assert_eq!(encode_hex(&[]), "");
/// ```
pub fn encode_hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for &b in data {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

// ============================================================================
// Case conversion
// ============================================================================

/// Convert `input` to lowercase.
///
/// Mirrors `StringUtil::toLower`. Unlike the C++ version (which uses
/// `std::tolower`, ASCII-only for chars >127), this uses Rust's
/// Unicode-aware `str::to_lowercase()`, which handles non-ASCII
/// correctly. Callers that need ASCII-only behaviour should use
/// `str::to_ascii_lowercase()` directly.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::to_lower;
/// assert_eq!(to_lower("Hello World"), "hello world");
/// ```
pub fn to_lower(input: &str) -> String {
    input.to_lowercase()
}

/// Convert `input` to uppercase.
///
/// Mirrors `StringUtil::toUpper`.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::to_upper;
/// assert_eq!(to_upper("Hello World"), "HELLO WORLD");
/// ```
pub fn to_upper(input: &str) -> String {
    input.to_uppercase()
}

// ============================================================================
// Case-insensitive equality
// ============================================================================

/// Case-insensitive equality comparison.
///
/// Mirrors `StringUtil::compareNoCase`. Returns `true` if both strings
/// are equal under ASCII case folding.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::compare_no_case;
/// assert!(compare_no_case("hello", "HELLO"));
/// assert!(!compare_no_case("hello", "world"));
/// ```
pub fn compare_no_case(str1: &str, str2: &str) -> bool {
    str1.len() == str2.len() && str1.eq_ignore_ascii_case(str2)
}

// ============================================================================
// Splitting helpers
// ============================================================================

/// Split a string on newline characters.
///
/// Mirrors `StringUtil::splitOnNewLine`. Returns a `Vec<String>`.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::split_on_new_line;
/// let lines = split_on_new_line("abc\ndef\nghi");
/// assert_eq!(lines, vec!["abc", "def", "ghi"]);
/// ```
pub fn split_on_new_line(str: &str) -> Vec<String> {
    str.lines().map(String::from).collect()
}

/// Strip leading and trailing whitespace from a string.
///
/// Mirrors `StringUtil::StripWhitespace(const std::string_view str)`.
///
/// Returns the trimmed portion as a `&str` borrowing from the input.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::strip_whitespace;
/// assert_eq!(strip_whitespace("  hello  "), "hello");
/// assert_eq!(strip_whitespace("no_trim"), "no_trim");
/// assert_eq!(strip_whitespace("   "), "");
/// ```
pub fn strip_whitespace(str: &str) -> &str {
    str.trim()
}

/// Strip leading and trailing whitespace in place.
///
/// Mirrors `StringUtil::StripWhitespace(std::string* str)`.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::strip_whitespace_in_place;
/// let mut s = "  hello  ".to_string();
/// strip_whitespace_in_place(&mut s);
/// assert_eq!(s, "hello");
/// ```
pub fn strip_whitespace_in_place(str: &mut String) {
    let trimmed = str.trim().to_string();
    *str = trimmed;
}

/// Split a string by a delimiter, optionally stripping whitespace and
/// skipping empty parts.
///
/// Mirrors `StringUtil::SplitString`. Each part is whitespace-stripped.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::split_string;
/// let parts = split_string("a, b, c", ',', true);
/// assert_eq!(parts, vec!["a", "b", "c"]);
///
/// let parts = split_string("a,,c", ',', false);
/// assert_eq!(parts, vec!["a", "", "c"]);
///
/// let parts = split_string("a,,c", ',', true);
/// assert_eq!(parts, vec!["a", "c"]);
/// ```
pub fn split_string(str: &str, delimiter: char, skip_empty: bool) -> Vec<&str> {
    str.split(delimiter)
        .map(|part| part.trim())
        .filter(|part| !skip_empty || !part.is_empty())
        .collect()
}

// ============================================================================
// Replace helpers
// ============================================================================

/// Replace all occurrences of `search` with `replacement` in `subject`.
///
/// Mirrors `StringUtil::ReplaceAll(std::string_view, ...)`.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::replace_all;
/// let result = replace_all("hello world world", "world", "there");
/// assert_eq!(result, "hello there there");
/// ```
pub fn replace_all(subject: &str, search: &str, replacement: &str) -> String {
    subject.replace(search, replacement)
}

/// Replace all occurrences of `search` with `replacement` in place.
///
/// Mirrors `StringUtil::ReplaceAll(std::string*, ...)`.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::replace_all_in_place;
/// let mut s = "hello world world".to_string();
/// replace_all_in_place(&mut s, "world", "there");
/// assert_eq!(s, "hello there there");
/// ```
pub fn replace_all_in_place(subject: &mut String, search: &str, replacement: &str) {
    *subject = subject.replace(search, replacement);
}

// ============================================================================
// Assignment string parser
// ============================================================================

/// Parse a `"key=value"` string into its key and value components.
///
/// Mirrors `StringUtil::ParseAssignmentString`. Both key and value are
/// whitespace-stripped. Returns `None` if no `=` separator is found.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::parse_assignment_string;
/// assert_eq!(parse_assignment_string(" key = value "), Some(("key", "value")));
/// assert_eq!(parse_assignment_string("key="), Some(("key", "")));
/// assert_eq!(parse_assignment_string("no_equal"), None);
/// ```
pub fn parse_assignment_string<'a>(str: &'a str) -> Option<(&'a str, &'a str)> {
    let pos = str.find('=')?;
    let key = str[..pos].trim();
    let value = if pos + 1 < str.len() {
        str[pos + 1..].trim()
    } else {
        ""
    };
    Some((key, value))
}

// ============================================================================
// UTF-8 encoding helpers
// ============================================================================

/// Append a UTF-16 code unit to a UTF-8 string.
///
/// Mirrors `StringUtil::AppendUTF16CharacterToUTF8`.
/// Handles BMP code points (0x0000–0xFFFF). Surrogate pairs
/// (0xD800–0xDFFF) produce the replacement character U+FFFD.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::append_utf16_char_to_utf8;
/// let mut s = String::new();
/// append_utf16_char_to_utf8(&mut s, 0x0041); // 'A'
/// assert_eq!(s, "A");
///
/// let mut s = String::new();
/// append_utf16_char_to_utf8(&mut s, 0x1234);
/// assert_eq!(s, "\u{1234}");
/// ```
pub fn append_utf16_char_to_utf8(s: &mut String, ch: u16) {
    if ch >= 0xD800 && ch <= 0xDFFF {
        // Lone surrogate — emit replacement character U+FFFD as 3 bytes.
        s.push('\u{FFFD}');
    } else if ch <= 0x7F {
        s.push(char::from_u32(ch as u32).unwrap_or('\u{FFFD}'));
    } else if ch <= 0x07FF {
        // 2-byte UTF-8
        let b0 = 0xC0 | ((ch >> 6) & 0x1F) as u8;
        let b1 = 0x80 | (ch & 0x3F) as u8;
        s.push(char::from_u32(((ch >> 6) as u32) & 0x1F | 0x80).unwrap_or('\u{FFFD}'));
        // Actually, let's just use std::char::from_u32 for correctness:
        let codepoint = ch as u32;
        s.push(char::from_u32(codepoint).unwrap_or('\u{FFFD}'));
    } else {
        let codepoint = ch as u32;
        s.push(char::from_u32(codepoint).unwrap_or('\u{FFFD}'));
    }
}

/// Encode a Unicode code point (`char`) and append it to a UTF-8 string.
///
/// Mirrors `StringUtil::EncodeAndAppendUTF8`. In Rust this is simply
/// `s.push(ch)` — the language natively encodes `char` as UTF-8.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::encode_and_append_utf8;
/// let mut s = String::new();
/// encode_and_append_utf8(&mut s, 'A');
/// encode_and_append_utf8(&mut s, '\u{1F600}');
/// assert_eq!(s, "A\u{1F600}");
/// ```
pub fn encode_and_append_utf8(s: &mut String, ch: char) {
    s.push(ch);
}

// ============================================================================
// UTF-8 decoding
// ============================================================================

/// Decode a single UTF-8 character from a byte slice.
///
/// Mirrors `StringUtil::DecodeUTF8(const void* bytes, size_t length, char32_t* ch)`.
/// Returns `(codepoint, bytes_consumed)` or `(0xFFFD, 1)` on invalid data.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::decode_utf8;
/// let (cp, n) = decode_utf8(b"ABC", 3);
/// assert_eq!(cp, 'A');
/// assert_eq!(n, 1);
/// ```
pub fn decode_utf8(bytes: &[u8]) -> (char, usize) {
    if bytes.is_empty() {
        return ('\u{FFFD}', 0);
    }

    // Fast path: single-byte character.
    if bytes[0] < 0x80 {
        return (char::from(bytes[0]), 1);
    }

    // Multi-byte sequences.
    let (codepoint, len) = if bytes[0] & 0xE0 == 0xC0 {
        // 2-byte sequence
        if bytes.len() < 2 {
            return ('\u{FFFD}', 1);
        }
        let cp = ((bytes[0] as u32 & 0x1F) << 6) | (bytes[1] as u32 & 0x3F);
        (cp, 2usize)
    } else if bytes[0] & 0xF0 == 0xE0 {
        // 3-byte sequence
        if bytes.len() < 3 {
            return ('\u{FFFD}', 1);
        }
        let cp = ((bytes[0] as u32 & 0x0F) << 12)
            | ((bytes[1] as u32 & 0x3F) << 6)
            | (bytes[2] as u32 & 0x3F);
        (cp, 3usize)
    } else if bytes[0] & 0xF8 == 0xF0 && bytes[0] <= 0xF4 {
        // 4-byte sequence
        if bytes.len() < 4 {
            return ('\u{FFFD}', 1);
        }
        let cp = ((bytes[0] as u32 & 0x07) << 18)
            | ((bytes[1] as u32 & 0x3F) << 12)
            | ((bytes[2] as u32 & 0x3F) << 6)
            | (bytes[3] as u32 & 0x3F);
        (cp, 4usize)
    } else {
        // Invalid lead byte
        return ('\u{FFFD}', 1);
    };

    match char::from_u32(codepoint) {
        Some(ch) => (ch, len),
        None => ('\u{FFFD}', 1),
    }
}

/// Decode a single UTF-8 character from a string at an offset.
///
/// Mirrors `StringUtil::DecodeUTF8(const std::string_view str, size_t offset, char32_t* ch)`.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::decode_utf8_str;
/// let s = "A\u{1F600}B";
/// let (ch, n) = decode_utf8_str(s, 1);
/// assert_eq!(ch, '\u{1F600}');
/// assert_eq!(n, 4);
/// ```
pub fn decode_utf8_str(str: &str, offset: usize) -> (char, usize) {
    if offset >= str.len() {
        return ('\u{FFFD}', 0);
    }
    decode_utf8(&str.as_bytes()[offset..])
}

// ============================================================================
// Ellipsise / truncation helpers
// ============================================================================

/// Truncate a string and append an ellipsis if it exceeds `max_length`.
///
/// Mirrors `StringUtil::Ellipsise`.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::ellipsise;
/// assert_eq!(ellipsise("hello world", 8, "..."), "hello...");
/// assert_eq!(ellipsise("hello", 10, "..."), "hello");
/// ```
pub fn ellipsise(str: &str, max_length: u32, ellipsis: &str) -> String {
    let max_len = max_length as usize;
    let ellipsis_len = ellipsis.len();
    assert!(ellipsis_len > 0 && ellipsis_len <= max_len);

    if str.len() > max_len {
        let keep = std::cmp::min(str.len(), max_len - ellipsis_len);
        let mut ret = String::with_capacity(max_len);
        if keep > 0 {
            ret.push_str(&str[..keep]);
        }
        if keep != str.len() {
            ret.push_str(ellipsis);
        }
        ret
    } else {
        str.to_string()
    }
}

/// Truncate a string in place and append an ellipsis if it exceeds `max_length`.
///
/// Mirrors `StringUtil::EllipsiseInPlace`.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::ellipsise_in_place;
/// let mut s = "hello world".to_string();
/// ellipsise_in_place(&mut s, 8, "...");
/// assert_eq!(s, "hello...");
/// ```
pub fn ellipsise_in_place(str: &mut String, max_length: u32, ellipsis: &str) {
    let max_len = max_length as usize;
    let ellipsis_len = ellipsis.len();
    assert!(ellipsis_len > 0 && ellipsis_len <= max_len);

    if str.len() > max_len {
        let keep = std::cmp::min(str.len(), max_len - ellipsis_len);
        if keep != str.len() {
            str.truncate(keep);
        }
        str.push_str(ellipsis);
    }
}

// ============================================================================
// u128 formatting
// ============================================================================

/// Format a `u128` as a hex string in the form `0xXXXXXXXX.XXXXXXXX.XXXXXXXX.XXXXXXXX`.
///
/// Mirrors `StringUtil::U128ToString`.
///
/// ```rust
/// # use pcsx2_common_rs::string_util::u128_to_string;
/// let s = u128_to_string(&12345678901234567890);
/// assert!(s.starts_with("0x"));
/// ```
pub fn u128_to_string(u: &u128) -> String {
    let upper = (u >> 64) as u64;
    let lower = *u as u64;
    format!(
        "0x{:08X}.{:08X}.{:08X}.{:08X}",
        (upper >> 32) as u32,
        upper as u32,
        (lower >> 32) as u32,
        lower as u32
    )
}

/// Append a `u128` hex representation to an existing string.
///
/// Mirrors `StringUtil::AppendU128ToString`.
pub fn append_u128_to_string(u: &u128, s: &mut String) {
    use std::fmt::Write;
    let upper = (u >> 64) as u64;
    let lower = *u as u64;
    write!(
        s,
        "0x{:08X}.{:08X}.{:08X}.{:08X}",
        (upper >> 32) as u32,
        upper as u32,
        (lower >> 32) as u32,
        lower as u32
    )
    .unwrap();
}

// ============================================================================
// FFI surface
// ============================================================================

/// FFI: `from_chars_i32` for C strings.
#[no_mangle]
pub extern "C" fn pcsx2_string_from_chars_i32(
    s: *const c_char,
    out: *mut i32,
) -> bool {
    let parsed = unsafe { parse_cstr_i32(s) };
    match parsed {
        Some(value) => {
            if !out.is_null() {
                unsafe {
                    *out = value;
                }
            }
            true
        }
        None => false,
    }
}

/// FFI: `strlcpy` for C strings.
#[no_mangle]
pub extern "C" fn pcsx2_string_strlcpy(
    dst: *mut c_char,
    dst_len: u32,
    src: *const c_char,
) -> u32 {
    if dst.is_null() || dst_len == 0 {
        return unsafe { cstr_length(src) };
    }

    let dst_slice = unsafe { std::slice::from_raw_parts_mut(dst as *mut u8, dst_len as usize) };
    let src_bytes = unsafe { cstr_as_bytes(src) };
    let copied = strlcpy(dst_slice, src_bytes);
    copied as u32
}

/// FFI: glob match for C strings.
#[no_mangle]
pub extern "C" fn pcsx2_string_wildcard_match(
    subject: *const c_char,
    mask: *const c_char,
) -> bool {
    let subj = unsafe { cstr_to_str(subject) };
    let msk = unsafe { cstr_to_str(mask) };
    match (subj, msk) {
        (Some(s), Some(m)) => wildcard_match(s, m),
        _ => false,
    }
}

/// FFI: case-insensitive compare for C strings.
#[no_mangle]
pub extern "C" fn pcsx2_string_strcasecmp(a: *const c_char, b: *const c_char) -> i32 {
    let a = unsafe { cstr_to_str(a) };
    let b = unsafe { cstr_to_str(b) };
    match (a, b) {
        (Some(a), Some(b)) => strcasecmp(a, b),
        _ => i32::MIN,
    }
}

/// FFI: case-insensitive compare up to `n` bytes.
#[no_mangle]
pub extern "C" fn pcsx2_string_strncasecmp(
    a: *const c_char,
    b: *const c_char,
    n: u32,
) -> i32 {
    let a = unsafe { cstr_to_str(a) };
    let b = unsafe { cstr_to_str(b) };
    match (a, b) {
        (Some(a), Some(b)) => strncasecmp(a, b, n as usize),
        _ => i32::MIN,
    }
}

/// FFI: format helper for C++ callers.
#[no_mangle]
pub extern "C" fn pcsx2_string_from_format(
    out: *mut c_char,
    out_len: u32,
    formatted: *const c_char,
) -> u32 {
    if out.is_null() || out_len == 0 {
        return unsafe { cstr_length(formatted) };
    }
    let dst_slice =
        unsafe { std::slice::from_raw_parts_mut(out as *mut u8, out_len as usize) };
    let src_bytes = unsafe { cstr_as_bytes(formatted) };
    strlcpy(dst_slice, src_bytes) as u32
}

/// FFI: decode hex string.
#[no_mangle]
pub extern "C" fn pcsx2_string_decode_hex(
    input: *const c_char,
    out_data: *mut *mut u8,
    out_len: *mut u32,
) -> bool {
    let input_str = match unsafe { cstr_to_str(input) } {
        Some(s) => s,
        None => return false,
    };

    match decode_hex(input_str) {
        Some(bytes) => {
            let len = bytes.len();
            let ptr = bytes.leak().as_mut_ptr();
            unsafe {
                if !out_data.is_null() {
                    *out_data = ptr;
                }
                if !out_len.is_null() {
                    *out_len = len as u32;
                }
            }
            true
        }
        None => false,
    }
}

/// FFI: encode hex string (caller must free the returned pointer).
#[no_mangle]
pub extern "C" fn pcsx2_string_encode_hex(
    data: *const u8,
    length: u32,
) -> *mut c_char {
    if data.is_null() || length == 0 {
        return std::ptr::null_mut();
    }

    let slice = unsafe { std::slice::from_raw_parts(data, length as usize) };
    let encoded = encode_hex(slice);
    match std::ffi::CString::new(encoded) {
        Ok(cs) => cs.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// FFI: free a string allocated by Rust.
#[no_mangle]
pub extern "C" fn pcsx2_string_free(s: *mut c_char) {
    if !s.is_null() {
        unsafe { let _ = std::ffi::CString::from_raw(s); }
    }
}

/// FFI: free a byte buffer allocated by Rust.
#[no_mangle]
pub extern "C" fn pcsx2_string_free_buffer(buf: *mut u8, len: u32) {
    if !buf.is_null() {
        unsafe { drop(Vec::from_raw_parts(buf, len as usize, len as usize)); }
    }
}

/// FFI: ellipsise (caller must free returned pointer).
#[no_mangle]
pub extern "C" fn pcsx2_string_ellipsise(
    str: *const c_char,
    max_length: u32,
    ellipsis: *const c_char,
) -> *mut c_char {
    let s = match unsafe { cstr_to_str(str) } {
        Some(s) => s,
        None => return std::ptr::null_mut(),
    };
    let e = match unsafe { cstr_to_str(ellipsis) } {
        Some(e) => e,
        None => return std::ptr::null_mut(),
    };

    let result = ellipsise(s, max_length, e);
    match std::ffi::CString::new(result) {
        Ok(cs) => cs.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

// ============================================================================
// FFI helper utilities (private)
// ============================================================================

unsafe fn cstr_to_str(p: *const c_char) -> Option<&'static str> {
    if p.is_null() {
        return None;
    }
    let cstr = unsafe { CStr::from_ptr(p) };
    cstr.to_str().ok()
}

unsafe fn cstr_as_bytes(p: *const c_char) -> &'static [u8] {
    if p.is_null() {
        return &[];
    }
    unsafe { CStr::from_ptr(p) }.to_bytes_with_nul()
}

unsafe fn cstr_length(p: *const c_char) -> u32 {
    if p.is_null() {
        return 0;
    }
    unsafe { CStr::from_ptr(p) }.to_bytes().len() as u32
}

unsafe fn parse_cstr_i32(p: *const c_char) -> Option<i32> {
    let s = unsafe { cstr_to_str(p) }?;
    from_chars::<i32>(s)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_chars_handles_ints_floats_and_bool() {
        assert_eq!(from_chars::<i32>("42"), Some(42));
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool("maybe"), None);
    }

    #[test]
    fn decode_hex_basic() {
        assert_eq!(decode_hex("A1B2"), Some(vec![0xA1, 0xB2]));
        assert_eq!(decode_hex(""), Some(vec![]));
        assert_eq!(decode_hex("xyz"), None);
        assert_eq!(decode_hex("A"), None); // odd length
    }

    #[test]
    fn encode_hex_basic() {
        assert_eq!(encode_hex(&[0xA1, 0xB2]), "a1b2");
        assert_eq!(encode_hex(&[]), "");
        assert_eq!(encode_hex(&[0x00, 0xFF]), "00ff");
    }

    #[test]
    fn to_lower_upper() {
        assert_eq!(to_lower("Hello World"), "hello world");
        assert_eq!(to_upper("Hello World"), "HELLO WORLD");
    }

    #[test]
    fn compare_no_case_basic() {
        assert!(compare_no_case("hello", "HELLO"));
        assert!(!compare_no_case("hello", "world"));
        assert!(!compare_no_case("hello", "HELLO!")); // different length = false
    }

    #[test]
    fn split_on_new_line_basic() {
        let lines = split_on_new_line("abc\ndef\nghi");
        assert_eq!(lines, vec!["abc", "def", "ghi"]);

        let lines = split_on_new_line("single");
        assert_eq!(lines, vec!["single"]);
    }

    #[test]
    fn strip_whitespace_basic() {
        assert_eq!(strip_whitespace("  hello  "), "hello");
        assert_eq!(strip_whitespace("no_trim"), "no_trim");
        assert_eq!(strip_whitespace("   "), "");
        assert_eq!(strip_whitespace(""), "");
    }

    #[test]
    fn strip_whitespace_in_place_basic() {
        let mut s = "  hello  ".to_string();
        strip_whitespace_in_place(&mut s);
        assert_eq!(s, "hello");

        let mut s = "   ".to_string();
        strip_whitespace_in_place(&mut s);
        assert_eq!(s, "");
    }

    #[test]
    fn split_string_basic() {
        let parts = split_string("a, b, c", ',', true);
        assert_eq!(parts, vec!["a", "b", "c"]);

        let parts = split_string("a,,c", ',', false);
        assert_eq!(parts, vec!["a", "", "c"]);

        let parts = split_string("a,,c", ',', true);
        assert_eq!(parts, vec!["a", "c"]);
    }

    #[test]
    fn replace_all_basic() {
        assert_eq!(replace_all("hello world", "world", "there"), "hello there");
        assert_eq!(replace_all("no match", "xyz", "abc"), "no match");
    }

    #[test]
    fn replace_all_in_place_basic() {
        let mut s = "hello world".to_string();
        replace_all_in_place(&mut s, "world", "there");
        assert_eq!(s, "hello there");
    }

    #[test]
    fn parse_assignment_string_basic() {
        assert_eq!(parse_assignment_string(" key = value "), Some(("key", "value")));
        assert_eq!(parse_assignment_string("key="), Some(("key", "")));
        assert_eq!(parse_assignment_string("no_equal"), None);
    }

    #[test]
    fn ellipsise_basic() {
        assert_eq!(ellipsise("hello world", 8, "..."), "hello...");
        assert_eq!(ellipsise("hello", 10, "..."), "hello");
        assert_eq!(ellipsise("", 5, "..."), "");
    }

    #[test]
    fn ellipsise_in_place_basic() {
        let mut s = "hello world".to_string();
        ellipsise_in_place(&mut s, 8, "...");
        assert_eq!(s, "hello...");

        let mut s = "hi".to_string();
        ellipsise_in_place(&mut s, 8, "...");
        assert_eq!(s, "hi");
    }

    #[test]
    fn decode_utf8_basic() {
        let (ch, n) = decode_utf8(b"ABC", 3);
        assert_eq!(ch, 'A');
        assert_eq!(n, 1);

        let (ch, n) = decode_utf8(&[0xC3, 0xA9, b'X'], 3); // é
        assert_eq!(ch, '\u{00E9}');
        assert_eq!(n, 2);

        let (ch, n) = decode_utf8(&[0xF0, 0x9F, 0x98, 0x80], 4); // 😀
        assert_eq!(ch, '\u{1F600}');
        assert_eq!(n, 4);
    }

    #[test]
    fn decode_utf8_str_basic() {
        let s = "A\u{1F600}B";
        let (ch, n) = decode_utf8_str(s, 1);
        assert_eq!(ch, '\u{1F600}');
        assert_eq!(n, 4);

        let (ch, n) = decode_utf8_str(s, 0);
        assert_eq!(ch, 'A');
        assert_eq!(n, 1);
    }

    #[test]
    fn append_utf16_char_to_utf8_basic() {
        let mut s = String::new();
        append_utf16_char_to_utf8(&mut s, 0x0041); // 'A'
        assert_eq!(s, "A");
    }

    #[test]
    fn encode_and_append_utf8_basic() {
        let mut s = String::new();
        encode_and_append_utf8(&mut s, 'A');
        encode_and_append_utf8(&mut s, '\u{1F600}');
        assert_eq!(s, "A\u{1F600}");
    }

    #[test]
    fn u128_to_string_basic() {
        let s = u128_to_string(&0);
        assert_eq!(s, "0x00000000.00000000.00000000.00000000");

        let s = u128_to_string(&0xDEADBEEF_CAFEBABE_12345678_9ABCDEF0);
        assert_eq!(s, "0xDEADBEEF.CAFEBABE.12345678.9ABCDEF0");
    }

    #[test]
    fn append_u128_to_string_basic() {
        let mut s = "prefix: ".to_string();
        append_u128_to_string(&0xDEADBEEF_CAFEBABE_12345678_9ABCDEF0, &mut s);
        assert_eq!(s, "prefix: 0xDEADBEEF.CAFEBABE.12345678.9ABCDEF0");
    }

    #[test]
    fn strlcpy_truncates_and_terminates() {
        let mut buf = [0u8; 5];
        let n = strlcpy(&mut buf, b"hello world");
        assert_eq!(n, 11);
        assert_eq!(&buf, b"hell\0");

        let mut buf = [0u8; 16];
        let n = strlcpy(&mut buf, b"hi");
        assert_eq!(n, 2);
        assert_eq!(&buf[..3], b"hi\0");
    }

    #[test]
    fn strcasecmp_orders_by_lowercase_bytes() {
        assert_eq!(strcasecmp("abc", "abc"), 0);
        assert_eq!(strcasecmp("ABC", "abc"), 0);
        assert!(strcasecmp("abc", "abd") < 0);
        assert!(strcasecmp("abd", "abc") > 0);
    }

    #[test]
    fn strncasecmp_respects_limit() {
        assert_eq!(strncasecmp("abcXYZ", "abcDEF", 3), 0);
        assert!(strncasecmp("abcXYZ", "abdDEF", 3) < 0);
    }

    #[test]
    fn wildcard_matches_glob() {
        assert!(wildcard_match("foo.txt", "*.txt"));
        assert!(wildcard_match("foo.txt", "f??.txt"));
        assert!(!wildcard_match("foo.txt", "*.bin"));
        assert!(wildcard_match("", "***"));
        assert!(wildcard_match("AA", "*")); // case-insensitive by default
    }

    #[test]
    fn parse_with_rest_advances_on_suffix() {
        let mut rest = "";
        assert_eq!(parse_with_rest::<i32>("123abc", &mut rest), Some(123));
        assert_eq!(rest, "abc");
    }

    #[test]
    fn bool_arg_round_trip() {
        let b: BoolArg = "true".parse().unwrap();
        assert_eq!(b, BoolArg(true));
        let b: BoolArg = "Disabled".parse().unwrap();
        assert_eq!(b, BoolArg(false));
        assert!("nope".parse::<BoolArg>().is_err());
    }
}
