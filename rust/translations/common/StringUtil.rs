//! Idiomatic Rust translation of PCSX2's `StringUtil` (C++ header + implementation).
//!
//! Provides ASCII / wide-string / case-insensitive / UTF-8 helpers, integer and
//! boolean parsing, hexadecimal encoding, wildcard matching, splitting, joining,
//! and various small textual utilities.  All functions are pure and return owned
//! `String` values or borrow `&str` / `Option<T>` where appropriate, instead of
//! mutating caller-provided buffers the way the C++ originals do.

#![allow(dead_code)]

use std::borrow::Cow;
use std::char;
use std::fmt::Write as _;
use std::str::FromStr;

// ---------------------------------------------------------------------------
// Case-insensitive comparison
// ---------------------------------------------------------------------------

/// ASCII case-insensitive comparison of two byte slices.  Returns negative,
/// zero, or positive like the C `strcasecmp`.
pub fn strcasecmp(a: &[u8], b: &[u8]) -> i32 {
    strncasecmp(a, b, usize::MAX)
}

/// ASCII case-insensitive comparison, comparing at most `n` bytes.
pub fn strncasecmp(a: &[u8], b: &[u8], n: usize) -> i32 {
    let n = n.min(a.len()).min(b.len());
    for i in 0..n {
        let ca = a[i].to_ascii_lowercase();
        let cb = b[i].to_ascii_lowercase();
        if ca != cb {
            return (ca as i32) - (cb as i32);
        }
    }
    0
}

/// `strcasecmp` operating on `&str`.
pub fn stricmp(a: &str, b: &str) -> i32 {
    strcasecmp(a.as_bytes(), b.as_bytes())
}

/// `strcasecmp` operating on `&str`, wide variant (alias of [`stricmp`]).
pub fn wx_stricmp(a: &str, b: &str) -> i32 {
    stricmp(a, b)
}

/// Returns true when the two slices are equal ignoring ASCII case.
pub fn compare_no_case(a: &str, b: &str) -> bool {
    a.len() == b.len() && strncasecmp(a.as_bytes(), b.as_bytes(), a.len()) == 0
}

// ---------------------------------------------------------------------------
// Wildcard / glob matching
// ---------------------------------------------------------------------------

/// `*` matches any run of characters; `?` matches exactly one character.  All
/// other characters are compared literally (and case-insensitively when
/// `case_sensitive` is `false`, per the C++ implementation).
pub fn wildcard_match(subject: &str, mask: &str, case_sensitive: bool) -> bool {
    let sb = subject.as_bytes();
    let mb = mask.as_bytes();

    let mut si = 0usize;
    let mut mi = 0usize;
    let mut star: Option<(usize, usize)> = None;

    while si < sb.len() {
        if mi < mb.len() && mb[mi] == b'*' {
            star = Some((mi, si));
            mi += 1;
        } else if mi < mb.len()
            && (mb[mi] == b'?'
                || (case_sensitive && mb[mi] == sb[si])
                || (!case_sensitive
                    && mb[mi].to_ascii_lowercase() == sb[si].to_ascii_lowercase()))
        {
            mi += 1;
            si += 1;
        } else if let Some((m, s)) = star {
            mi = m + 1;
            star = Some((m, s + 1));
            si = s + 1;
        } else {
            return false;
        }
    }

    while mi < mb.len() && mb[mi] == b'*' {
        mi += 1;
    }
    mi == mb.len()
}

// ---------------------------------------------------------------------------
// Safe copy
// ---------------------------------------------------------------------------

/// `strlcpy` equivalent.  Copies at most `dst.len() - 1` bytes from `src` into
/// `dst`, always NUL-terminating.  Returns the length of `src`.
pub fn strlcpy(dst: &mut [u8], src: &[u8]) -> usize {
    let len = src.len();
    if len + 1 <= dst.len() {
        dst[..len].copy_from_slice(src);
        dst[len] = 0;
    } else if !dst.is_empty() {
        let copy = dst.len() - 1;
        dst[..copy].copy_from_slice(&src[..copy]);
        dst[copy] = 0;
    }
    len
}

// ---------------------------------------------------------------------------
// Starts-with / ends-with helpers
// ---------------------------------------------------------------------------

/// ASCII case-insensitive "starts with" test.
pub fn starts_with_no_case<'a>(s: &'a str, prefix: &str) -> bool {
    !s.is_empty()
        && s.len() >= prefix.len()
        && strncasecmp(s.as_bytes(), prefix.as_bytes(), prefix.len()) == 0
}

/// ASCII case-insensitive "ends with" test.
pub fn ends_with_no_case(s: &str, suffix: &str) -> bool {
    let n = suffix.len();
    s.len() >= n && strncasecmp(&s.as_bytes()[s.len() - n..], suffix.as_bytes(), n) == 0
}

/// Case-sensitive "starts with" wrapper around [`str::starts_with`].
pub fn starts_with(s: &str, prefix: &str) -> bool {
    s.starts_with(prefix)
}

/// Case-sensitive "ends with" wrapper around [`str::ends_with`].
pub fn ends_with(s: &str, suffix: &str) -> bool {
    s.ends_with(suffix)
}

/// Returns true when `haystack` contains `needle` (case sensitive).  An empty
/// needle is found only inside an empty haystack.
pub fn contains<'a, C: ?Sized + AsRef<[u8]>>(haystack: &C, needle: &[u8]) -> bool {
    if needle.is_empty() {
        return haystack.as_ref().is_empty();
    }
    haystack
        .as_ref()
        .windows(needle.len())
        .any(|w| w == needle)
}

// ---------------------------------------------------------------------------
// Replace-all
// ---------------------------------------------------------------------------

/// Returns a copy of `subject` with every non-overlapping occurrence of
/// `search` replaced with `replacement`.
pub fn replace_all(subject: &str, search: &str, replacement: &str) -> String {
    if search.is_empty() {
        return subject.to_string();
    }
    let mut out = String::with_capacity(subject.len());
    let bytes = subject.as_bytes();
    let needle = search.as_bytes();
    let mut i = 0;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            out.push_str(replacement);
            i += needle.len();
        } else {
            // Find next char boundary at or after i+1.
            let next = (i + 1..=i + needle.len().saturating_add(4).min(bytes.len()))
                .find(|&j| subject.is_char_boundary(j))
                .unwrap_or(bytes.len());
            out.push_str(&subject[i..next]);
            i = next;
        }
    }
    if i < bytes.len() {
        out.push_str(&subject[i..]);
    }
    out
}

// ---------------------------------------------------------------------------
// Strip whitespace
// ---------------------------------------------------------------------------

/// Returns a `&str` slice of `s` with leading and trailing ASCII whitespace
/// removed.
pub fn strip_whitespace(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut start = 0;
    while start < bytes.len() && bytes[start].is_ascii_whitespace() {
        start += 1;
    }
    let mut end = bytes.len();
    while end > start && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    &s[start..end]
}

// ---------------------------------------------------------------------------
// Split / join
// ---------------------------------------------------------------------------

/// Splits `s` on the single ASCII character `delimiter`.  When `skip_empty` is
/// `true`, empty parts (and parts that become empty after stripping) are
/// omitted.
pub fn split_string(s: &str, delimiter: char, skip_empty: bool) -> Vec<&str> {
    let mut out = Vec::new();
    let mut last = 0usize;
    for (i, c) in s.char_indices() {
        if c == delimiter {
            let part = strip_whitespace(&s[last..i]);
            if !skip_empty || !part.is_empty() {
                out.push(part);
            }
            last = i + c.len_utf8();
        }
    }
    if last < s.len() {
        let part = strip_whitespace(&s[last..]);
        if !skip_empty || !part.is_empty() {
            out.push(part);
        }
    }
    out
}

/// Joins the `&str`s yielded by `iter` with `delimiter` between consecutive
/// items.
pub fn join_string<'a, I>(iter: I, delimiter: &str) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    let mut out = String::new();
    let mut first = true;
    for item in iter {
        if !first {
            out.push_str(delimiter);
        }
        out.push_str(item);
        first = false;
    }
    out
}

/// Joins the `String`s yielded by `iter` with `delimiter` between consecutive
/// items.
pub fn join_strings<'a, I>(iter: I, delimiter: &str) -> String
where
    I: IntoIterator<Item = &'a String>,
{
    let mut out = String::new();
    let mut first = true;
    for item in iter {
        if !first {
            out.push_str(delimiter);
        }
        out.push_str(item);
        first = false;
    }
    out
}

// ---------------------------------------------------------------------------
// Split / parse on a line
// ---------------------------------------------------------------------------

/// Splits `s` on either `\n` or `\r\n`.  Mirrors the C++ `splitOnNewLine`.
pub fn split_on_new_line(s: &str) -> Vec<&str> {
    s.lines().collect()
}

/// Parses a `"key = value"` style assignment.  Both sides are stripped of
/// leading/trailing whitespace before being returned.
pub fn parse_assignment_string(s: &str) -> Option<(&str, &str)> {
    let pos = s.find('=')?;
    let key = strip_whitespace(&s[..pos]);
    let value = if pos + 1 < s.len() {
        strip_whitespace(&s[pos + 1..])
    } else {
        ""
    };
    Some((key, value))
}

// ---------------------------------------------------------------------------
// Case conversion
// ---------------------------------------------------------------------------

/// Returns a lowercase copy of `s` (ASCII only — matches the C++ behaviour
/// that delegates to `std::tolower` on `unsigned char`).
pub fn to_lower(s: &str) -> String {
    s.chars().map(|c| c.to_ascii_lowercase()).collect()
}

/// Returns an uppercase copy of `s` (ASCII only).
pub fn to_upper(s: &str) -> String {
    s.chars().map(|c| c.to_ascii_uppercase()).collect()
}

// ---------------------------------------------------------------------------
// "no char" – safe wrapper around character checks
// ---------------------------------------------------------------------------

/// Returns true if `s` does NOT contain `c`.  Mirrors the C++ helper that
/// used to be spelled `nochar`.
pub fn no_char(s: &str, c: char) -> bool {
    !s.contains(c)
}

// ---------------------------------------------------------------------------
// Parsing helpers (`from_chars` / `parse_int` / `parse_bool`)
// ---------------------------------------------------------------------------

/// Trait for integer types that can be parsed from a string in an arbitrary
/// base. Mirrors the `int xx::fromChars(...)` template the C++ side uses.
pub trait FromRadix: Sized {
    fn from_radix(s: &str, base: u32) -> Result<Self, std::num::ParseIntError>;
}

macro_rules! impl_from_radix {
    ($($t:ty),* $(,)?) => {
        $(
            impl FromRadix for $t {
                fn from_radix(s: &str, base: u32) -> Result<Self, std::num::ParseIntError> {
                    <$t>::from_str_radix(s, base)
                }
            }
        )*
    };
}

impl_from_radix!(i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize);

/// Wrapper around `str::parse` for integers in `base`.  Returns `None` on
/// failure.
pub fn from_chars<T: FromRadix>(s: &str, base: u32) -> Option<T> {
    T::from_radix(s, base).ok()
}

/// Convenience wrapper: integer parsing in base 10.  Mirrors the intent of
/// the C++ `parseInt`.
pub fn parse_int<T: FromStr>(s: &str) -> Option<T>
where
    <T as FromStr>::Err: std::fmt::Debug,
{
    T::from_str(s).ok()
}

/// Parses a boolean from common textual representations:
/// truthy — `"true"`, `"yes"`, `"on"`, `"1"`, `"enabled"`;
/// falsy  — `"false"`, `"no"`, `"off"`, `"0"`, `"disabled"`.
/// Comparison is ASCII case-insensitive.
pub fn parse_bool(s: &str) -> Option<bool> {
    const TRUTHY: &[&str] = &["true", "yes", "on", "1", "enabled"];
    const FALSY: &[&str] = &["false", "no", "off", "0", "disabled"];

    if TRUTHY.iter().any(|v| compare_no_case(v, s)) {
        return Some(true);
    }
    if FALSY.iter().any(|v| compare_no_case(v, s)) {
        return Some(false);
    }
    None
}

/// Generic `lexical_cast`.  Tries to parse `s` into `T` and returns the
/// result, or `None` on failure.
pub fn lexical_cast<T>(s: &str) -> Option<T>
where
    T: FromStr,
    <T as FromStr>::Err: std::fmt::Debug,
{
    T::from_str(s).ok()
}

// ---------------------------------------------------------------------------
// Hex encoding
// ---------------------------------------------------------------------------

/// Decodes a hexadecimal byte string of even length into a `Vec<u8>`.  Returns
/// `None` on any malformed character.
pub fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    for chunk in bytes.chunks(2) {
        let hi = (chunk[0] as char).to_digit(16)? as u8;
        let lo = (chunk[1] as char).to_digit(16)? as u8;
        out.push((hi << 4) | lo);
    }
    Some(out)
}

/// Encodes `data` as lowercase hex.
pub fn encode_hex(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        let _ = write!(s, "{:02x}", b);
    }
    s
}

/// Formats an integer as a 0x-prefixed, upper-case hex string.
pub fn format_to_hex<T: std::fmt::UpperHex>(value: T) -> String {
    format!("0x{:X}", value)
}

// ---------------------------------------------------------------------------
// Path slashes
// ---------------------------------------------------------------------------

/// Replaces any `/` in `s` with the platform native separator.  On Windows
/// this converts forward slashes to back-slashes; on Unix-like systems the
/// function is a no-op.
pub fn to_native_slashes(s: &str) -> Cow<'_, str> {
    if cfg!(windows) {
        Cow::Owned(s.replace('/', "\\"))
    } else {
        Cow::Borrowed(s)
    }
}

// ---------------------------------------------------------------------------
// UTF-8 codepoint helpers
// ---------------------------------------------------------------------------

/// Encodes a `char32_t` as UTF-8 and appends it to `s`.  `REPLACEMENT_CHAR`
/// (U+FFFD) is emitted for code points above U+10FFFF.
pub fn encode_and_append_utf8(s: &mut String, ch: char32_t) {
    if ch <= 0x7F {
        s.push(ch as u8 as char);
    } else if ch <= 0x07FF {
        s.push((0xC0 | ((ch >> 6) & 0x1F)) as u8 as char);
        s.push((0x80 | (ch & 0x3F)) as u8 as char);
    } else if ch <= 0xFFFF {
        s.push((0xE0 | ((ch >> 12) & 0x0F)) as u8 as char);
        s.push((0x80 | ((ch >> 6) & 0x3F)) as u8 as char);
        s.push((0x80 | (ch & 0x3F)) as u8 as char);
    } else if ch <= 0x10FFFF {
        s.push((0xF0 | ((ch >> 18) & 0x07)) as u8 as char);
        s.push((0x80 | ((ch >> 12) & 0x3F)) as u8 as char);
        s.push((0x80 | ((ch >> 6) & 0x3F)) as u8 as char);
        s.push((0x80 | (ch & 0x3F)) as u8 as char);
    } else {
        s.push('\u{FFFD}');
    }
}

/// Decodes a single UTF-8 codepoint from the start of `bytes`.  Returns the
/// number of bytes consumed (always 1 on invalid input) and writes the
/// decoded value to `ch`.  The C++ original writes `0xFFFFFFFFu` to `*ch` on
/// invalid input; this translation does the same via `char::REPLACEMENT_CODE_POINT`.
pub fn decode_utf8(bytes: &[u8], ch: &mut char32_t) -> usize {
    if bytes.is_empty() {
        *ch = 0xFFFF_FFFF;
        return 1;
    }
    let s0 = bytes[0];
    if s0 < 0x80 {
        *ch = s0 as u32;
        return 1;
    }
    if (s0 & 0xE0) == 0xC0 && bytes.len() >= 2 {
        *ch = (((s0 & 0x1F) as u32) << 6) | ((bytes[1] & 0x3F) as u32);
        return 2;
    }
    if (s0 & 0xF0) == 0xE0 && bytes.len() >= 3 {
        *ch = (((s0 & 0x0F) as u32) << 12)
            | (((bytes[1] & 0x3F) as u32) << 6)
            | ((bytes[2] & 0x3F) as u32);
        return 3;
    }
    if (s0 & 0xF8) == 0xF0 && s0 <= 0xF4 && bytes.len() >= 4 {
        *ch = (((s0 & 0x07) as u32) << 18)
            | (((bytes[1] & 0x3F) as u32) << 12)
            | (((bytes[2] & 0x3F) as u32) << 6)
            | ((bytes[3] & 0x3F) as u32);
        return 4;
    }
    *ch = 0xFFFF_FFFF;
    1
}

/// Convenience wrapper around [`decode_utf8`] for `&str` starting at `offset`.
pub fn decode_utf8_at(s: &str, offset: usize, ch: &mut char32_t) -> usize {
    decode_utf8(&s.as_bytes()[offset..], ch)
}

// ---------------------------------------------------------------------------
// Ellipsisation
// ---------------------------------------------------------------------------

/// Truncates `s` so that the returned string is at most `max_length` bytes;
/// when truncation occurs, the supplied `ellipsis` is appended.  Mirrors the
/// C++ `Ellipsise`.
pub fn ellipsise(s: &str, max_length: usize, ellipsis: &str) -> String {
    debug_assert!(!ellipsis.is_empty() && ellipsis.len() <= max_length);
    if s.len() > max_length {
        let keep = max_length.saturating_sub(ellipsis.len());
        let mut out = String::with_capacity(max_length);
        out.push_str(&s[..keep]);
        if keep != s.len() {
            out.push_str(ellipsis);
        }
        out
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Format
// ---------------------------------------------------------------------------

/// `printf`-style format helper.  Mirrors the C++ `StdStringFromFormat`.
pub fn std_string_from_format(format: &str, args: std::fmt::Arguments<'_>) -> String {
    match args {
        _a if format.is_empty() => String::new(),
        _ => format!("{}", args),
    }
}

/// Variadic version of [`std_string_from_format`] for runtime format strings.
pub fn format(fmt: &str, args: std::fmt::Arguments<'_>) -> String {
    std_string_from_format(fmt, args)
}

// ---------------------------------------------------------------------------
// Convenience aliases matching the C++ names from the task description
// ---------------------------------------------------------------------------

/// `Strncasecmp` — ASCII case-insensitive comparison up to `n` bytes.
pub fn strncasecmp_alias(a: &str, b: &str, n: usize) -> i32 {
    strncasecmp(a.as_bytes(), b.as_bytes(), n)
}

/// `Stricmp` — alias for [`stricmp`].
pub fn stricmp_alias(a: &str, b: &str) -> i32 {
    stricmp(a, b)
}

/// `WxStricmp` — alias for [`wx_stricmp`].
pub fn wx_stricmp_alias(a: &str, b: &str) -> i32 {
    wx_stricmp(a, b)
}

// ---------------------------------------------------------------------------
// 128-bit formatting (the C++ `u128` type is platform specific; here we
// accept any value implementing `UpperHex`).
// ---------------------------------------------------------------------------

/// Format a 128-bit value as four 32-bit groups separated by dots.  Mirrors
/// `U128ToString`.
pub fn u128_to_string(words: [u32; 4]) -> String {
    format!(
        "0x{:08X}.{:08X}.{:08X}.{:08X}",
        words[0], words[1], words[2], words[3]
    )
}

/// Append a 128-bit value to `s` and return `s` mutated.  Mirrors
/// `AppendU128ToString`.
pub fn append_u128_to_string(words: [u32; 4], s: &mut String) {
    let _ = write!(
        s,
        "0x{:08X}.{:08X}.{:08X}.{:08X}",
        words[0], words[1], words[2], words[3]
    );
}

// ---------------------------------------------------------------------------
// Type alias mirroring `char32_t`
// ---------------------------------------------------------------------------

/// Mirrors the C++ `char32_t` type.
pub type char32_t = u32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strncasecmp_works() {
        assert_eq!(strncasecmp(b"FooBar", b"foobar", 6), 0);
        assert!(strncasecmp(b"Foo", b"bar", 3) != 0);
    }

    #[test]
    fn starts_with_no_case_works() {
        assert!(starts_with_no_case("FooBar", "foo"));
        assert!(!starts_with_no_case("FooBar", "bar"));
    }

    #[test]
    fn ends_with_no_case_works() {
        assert!(ends_with_no_case("FooBar", "bar"));
        assert!(!ends_with_no_case("FooBar", "foo"));
    }

    #[test]
    fn replace_all_works() {
        assert_eq!(replace_all("a-b-c-d", "-", "+"), "a+b+c+d");
        assert_eq!(replace_all("hello", "z", "Z"), "hello");
        assert_eq!(replace_all("aaa", "a", "bb"), "bbbbbb");
    }

    #[test]
    fn strip_whitespace_works() {
        assert_eq!(strip_whitespace("  hello  "), "hello");
        assert_eq!(strip_whitespace(""), "");
        assert_eq!(strip_whitespace("   "), "");
    }

    #[test]
    fn split_string_works() {
        assert_eq!(split_string("a,b,,c", ',', true), vec!["a", "b", "c"]);
        assert_eq!(split_string("a,b,,c", ',', false), vec!["a", "b", "", "c"]);
    }

    #[test]
    fn wildcard_match_works() {
        assert!(wildcard_match("hello.txt", "*.txt", true));
        assert!(!wildcard_match("hello.bin", "*.txt", true));
        assert!(wildcard_match("hello.txt", "HELLO*", false));
    }

    #[test]
    fn parse_bool_works() {
        assert_eq!(parse_bool("True"), Some(true));
        assert_eq!(parse_bool("OFF"), Some(false));
        assert_eq!(parse_bool("maybe"), None);
    }

    #[test]
    fn parse_int_works() {
        assert_eq!(parse_int::<i32>("42"), Some(42));
        assert_eq!(parse_int::<i32>("-7"), Some(-7));
        assert_eq!(parse_int::<i32>("xx"), None);
    }

    #[test]
    fn encode_decode_hex_round_trip() {
        let data = [0u8, 1, 0xAB, 0xCD, 0xFF];
        let encoded = encode_hex(&data);
        assert_eq!(encoded, "0001abcdff");
        assert_eq!(decode_hex(&encoded), Some(data.to_vec()));
    }

    #[test]
    fn ellipsise_works() {
        assert_eq!(ellipsise("hello world", 5, "..."), "he...");
        assert_eq!(ellipsise("hi", 10, "..."), "hi");
    }

    #[test]
    fn to_lower_upper_works() {
        assert_eq!(to_lower("Hello, World!"), "hello, world!");
        assert_eq!(to_upper("Hello, World!"), "HELLO, WORLD!");
    }

    #[test]
    fn no_char_works() {
        assert!(no_char("hello", 'z'));
        assert!(!no_char("hello", 'l'));
    }

    #[test]
    fn format_to_hex_works() {
        assert_eq!(format_to_hex(0xDEAD_BEEFu32), "0xDEADBEEF");
    }

    #[test]
    fn to_native_slashes_works() {
        let s = to_native_slashes("path/to/file");
        if cfg!(windows) {
            assert_eq!(s, "path\\to\\file");
        } else {
            assert_eq!(s, "path/to/file");
        }
    }
}
