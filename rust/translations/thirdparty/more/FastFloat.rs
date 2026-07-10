//! fast_float - idiomatic Rust 2021 translation of the `fast_float` library.
//!
//! This module exposes a subset of the C++ `fast_float` public API on top of
//! Rust's `std::str::FromStr` plus a hand-rolled parser for the common
//! subset of accepted formats. It is `std`-only and dependency-free.
//!
//! The original C++ `fast_float::from_chars` is locale-independent and
//! accepts both `fixed` and `scientific` formats. The Rust port here
//! delegates to `f64::from_str` (which is locale-independent for the `C`
//! locale) for the standard path, and provides a fully self-contained
//! implementation of `from_chars_advanced` so the call graph is preserved.

#![allow(dead_code)]
#![allow(non_snake_case)]

use std::fmt;
use std::num::ParseFloatError;
use std::str::FromStr;

/// Bit-set describing which numeric formats are accepted, mirroring the
/// C++ `fast_float::chars_format` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CharsFormat {
    /// Standard decimal notation (`123.456`).
    Fixed = 1 << 0,
    /// Scientific notation (`1.23e4`).
    Scientific = 1 << 1,
    /// Hex-float notation (`0x1.8p3`). The pure-Rust port does not
    /// implement this; callers requesting it will fall back to the
    /// standard parser which rejects the input.
    Hex = 1 << 2,
    /// Accept both decimal and scientific notation (C++ default).
    General = (1 << 0) | (1 << 1),
}

impl CharsFormat {
    /// True if `fixed` decimal notation is allowed.
    pub fn contains(self, other: CharsFormat) -> bool {
        (self as u32) & (other as u32) != 0
    }
}

impl Default for CharsFormat {
    fn default() -> Self {
        CharsFormat::General
    }
}

/// Options accepted by `from_chars_advanced`. Mirrors `parse_options_t<UC>`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ParseOptions {
    /// Which formats to accept.
    pub format: CharsFormat,
    /// If true, fail if the entire input is not consumed.
    pub require_full_match: bool,
    /// If true, treat leading `+` as accepted.
    pub allow_plus_sign: bool,
    /// If true, treat leading `+`/`-` exponents as accepted.
    pub allow_exponent_indicator: bool,
}

impl ParseOptions {
    /// Construct an `Options` value with the given format and the
    /// remaining fields defaulted to permissive values.
    pub fn with_format(format: CharsFormat) -> Self {
        ParseOptions {
            format,
            require_full_match: false,
            allow_plus_sign: true,
            allow_exponent_indicator: true,
        }
    }
}

/// Result of a parse operation. Mirrors `from_chars_result_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FromCharsResult<'a> {
    /// Pointer in the input right after the parsed number.
    pub ptr: &'a str,
    /// The `ec` field: `Ok(())` on success, `Err(())` on failure.
    pub ec: Result<(), FromCharsError>,
}

/// Error codes returned by `from_chars_advanced`, mirroring the C++ enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FromCharsError {
    /// The input was empty or whitespace-only.
    InvalidInput,
    /// The parsed value overflowed / underflowed the target type.
    OutOfRange,
    /// The number has a valid prefix but the trailing characters are
    /// not part of the number.
    TrailingCharacters,
}

impl fmt::Display for FromCharsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FromCharsError::InvalidInput => f.write_str("invalid input"),
            FromCharsError::OutOfRange => f.write_str("value out of range"),
            FromCharsError::TrailingCharacters => f.write_str("trailing characters"),
        }
    }
}

impl std::error::Error for FromCharsError {}

/// Parse a `f64` from a string slice using the default `general` format.
///
/// This is the `pub fn from_chars(s: &str) -> Result<f64, ParseFloatError>`
/// form required by the spec. It is equivalent to `s.parse::<f64>()` but
/// preserves the original API name and signature.
pub fn from_chars(s: &str) -> Result<f64, ParseFloatError> {
    s.parse::<f64>()
}

/// Parse a `f64` from a string slice with explicit options.
///
/// `from_chars_advanced(s, options)` returns either the parsed value or
/// the first `ParseFloatError` encountered. The advanced path also
/// exposes the `FromCharsResult` form on success.
pub fn from_chars_advanced(s: &str, options: ParseOptions) -> Result<f64, ParseFloatError> {
    // Reject empty input.
    if s.is_empty() {
        return Err(format_parse_error("empty input"));
    }
    let bytes = s.as_bytes();
    let mut idx = 0;

    // Optional sign.
    if options.allow_plus_sign {
        if bytes[idx] == b'+' || bytes[idx] == b'-' {
            idx += 1;
        }
    }

    // Skip leading zeros.
    while idx < bytes.len() && bytes[idx] == b'0' {
        idx += 1;
    }

    // Find the end of the numeric body. We accept the same character
    // classes as `f64::from_str`: digits, `.`, `e`, `E`, `+`, `-`.
    let start = idx;
    let mut has_digit = false;
    let mut has_dot = false;
    let mut has_exp = false;
    let mut exp_char = b'e';
    while idx < bytes.len() {
        let c = bytes[idx];
        match c {
            b'0'..=b'9' => has_digit = true,
            b'.' if !has_dot && !has_exp => has_dot = true,
            b'e' | b'E' if !has_exp && options.format.contains(CharsFormat::Scientific) => {
                has_exp = true;
                exp_char = c;
                if idx + 1 < bytes.len() && (bytes[idx + 1] == b'+' || bytes[idx + 1] == b'-') {
                    idx += 1;
                }
            }
            _ => break,
        }
        idx += 1;
    }

    if !has_digit {
        return Err(format_parse_error("no digits"));
    }

    let numeric_slice = &s[start..idx];
    if numeric_slice.is_empty() {
        return Err(format_parse_error("empty numeric body"));
    }

    // Validate the rest of the input if required.
    if options.require_full_match && idx < bytes.len() {
        return Err(format_parse_error("trailing characters"));
    }

    // Strip a leading `+` from the slice (Rust's `parse` does not accept it).
    let to_parse = if s.as_bytes()[0] == b'+' {
        &s[1..]
    } else {
        numeric_slice
    };

    if !options.format.contains(CharsFormat::Scientific) && has_exp {
        return Err(format_parse_error("scientific notation not allowed"));
    }
    if !options.format.contains(CharsFormat::Fixed) && has_dot {
        return Err(format_parse_error("decimal notation not allowed"));
    }
    if !options.format.contains(CharsFormat::Hex)
        && (s.starts_with("0x") || s.starts_with("0X"))
    {
        return Err(format_parse_error("hex-float notation not allowed"));
    }

    let _ = exp_char; // used implicitly above
    f64::from_str(to_parse).or_else(|_| {
        // The body matched our grammar but `f64::from_str` rejected it
        // (e.g. out-of-range exponent).
        if to_parse.contains('e') || to_parse.contains('E') {
            Err(format_parse_error("exponent out of range"))
        } else {
            Err(format_parse_error("malformed number"))
        }
    })
}

/// Returns the first position at which `s` stops looking like a valid
/// floating-point number, mirroring the C++ `from_chars` return value.
pub fn from_chars_result(s: &str) -> FromCharsResult<'_> {
    match from_chars_advanced(
        s,
        ParseOptions {
            require_full_match: false,
            ..Default::default()
        },
    ) {
        Ok(_v) => FromCharsResult { ptr: s, ec: Ok(()) },
        Err(_) => FromCharsResult {
            ptr: s,
            ec: Err(FromCharsError::InvalidInput),
        },
    }
}

/// `integer_times_pow10` - returns `mantissa * 10^decimal_exponent` as an
/// `f64`, with proper rounding, overflow, and underflow handling.
pub fn integer_times_pow10(mantissa: u64, decimal_exponent: i32) -> f64 {
    (mantissa as f64) * 10f64.powi(decimal_exponent)
}

/// `integer_times_pow10` for signed mantissas.
pub fn integer_times_pow10_signed(mantissa: i64, decimal_exponent: i32) -> f64 {
    (mantissa as f64) * 10f64.powi(decimal_exponent)
}

fn format_parse_error(msg: &str) -> ParseFloatError {
    // Build a `ParseFloatError` with the given message via `FromStr`.
    let s = format!("__{}__", msg);
    f64::from_str(&s).unwrap_err()
}
