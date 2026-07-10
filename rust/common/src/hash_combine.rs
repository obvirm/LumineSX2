// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! boost::hash_combine-style seed mixing.
//!
//! Mirrors PCSX2's `common/HashCombine.h`. The C++ version is a variadic
//! template that folds each value into the seed using
//!
//! ```text
//! seed ^= hash(v) + 0x9e3779b9 + (seed << 6) + (seed >> 2)
//! ```
//!
//! recursively mixing any remaining arguments.
//!
//! The Rust port exposes three layers:
//!
//! - [`pcsx2_hash_combine_mix`] — pure-Rust helper. Generic over `T: Hash`,
//!   iterates a slice and folds each element through the same formula
//!   using `std::collections::hash_map::DefaultHasher` for type-erasure.
//! - [`hash_combine!`] — `#[macro_export]` declarative macro that
//!   reproduces the C++ variadic ergonomics: `hash_combine!(seed, a, b, c)`
//!   returns a new seed.
//! - Three `#[no_mangle] pub extern "C"` FFI exports (`u32` / `u64` /
//!   null-terminated C string) that wrap the helper for the C++ side.

use std::collections::hash_map::DefaultHasher;
use std::ffi::CStr;
use std::hash::{Hash, Hasher};
use std::os::raw::c_char;

/// Boost-style constant used in the mixing step. Identical to the value
/// used by `boost::hash_combine`, `folly::hash`, and PCSX2's
/// `HashCombine.h`.
const GOLDEN_RATIO: u64 = 0x9e37_79b9;

/// Fold a slice of hashable values into `seed` using the boost-style
/// mixing formula:
///
/// ```text
/// seed ^= hash(v) + GOLDEN_RATIO + (seed << 6) + (seed >> 2)
/// ```
///
/// Equivalent to repeated invocations of the C++ `HashCombine` variadic
/// template. Each value is dispatched through the [`Hash`] trait using a
/// fresh [`DefaultHasher`] for type erasure. All arithmetic wraps modulo
/// `2^64` to mirror the C++ `std::size_t` semantics.
#[inline]
pub fn pcsx2_hash_combine_mix<T: Hash>(seed: &mut u64, values: &[T]) {
    for v in values {
        let mut hasher = DefaultHasher::new();
        v.hash(&mut hasher);
        let h = hasher.finish();
        let mixed = h
            .wrapping_add(GOLDEN_RATIO)
            .wrapping_add(seed.wrapping_shl(6))
            .wrapping_add(seed.wrapping_shr(2));
        *seed ^= mixed;
    }
}

/// boost::hash_combine-style variadic seed mixer.
///
/// Mirrors the C++ template `HashCombine(seed, v1, v2, ...)`. The seed
/// expression is consumed by value and the resulting folded seed is
/// returned; the caller's `seed` is not mutated in place, matching Rust
/// expression-macro ergonomics.
///
/// ```ignore
/// let seed = hash_combine!(0u64, "foo", 42_u32);
/// ```
#[macro_export]
macro_rules! hash_combine {
    ($seed:expr, $($v:expr),+) => {{
        let mut __seed = $seed;
        $crate::hash_combine::pcsx2_hash_combine_mix(&mut __seed, &[$($v),+]);
        __seed
    }};
}

// ---------------------------------------------------------------------------
// FFI surface (consumed by C++ PCSX2 via cbindgen).
//
// Each wrapper funnels through `pcsx2_hash_combine_mix` so the mixing
// algorithm lives in exactly one place and the C++ side sees the same
// numeric output for a given (seed, value) pair across all three overloads.
// ---------------------------------------------------------------------------

/// FFI export: fold a `uint32_t` into the seed.
#[no_mangle]
pub extern "C" fn pcsx2_hash_combine_u32(seed: u64, value: u32) -> u64 {
    let mut s = seed;
    pcsx2_hash_combine_mix(&mut s, &[value]);
    s
}

/// FFI export: fold a `uint64_t` into the seed.
#[no_mangle]
pub extern "C" fn pcsx2_hash_combine_u64(seed: u64, value: u64) -> u64 {
    let mut s = seed;
    pcsx2_hash_combine_mix(&mut s, &[value]);
    s
}

/// FFI export: fold a null-terminated C string into the seed.
///
/// The string's bytes (excluding the trailing NUL terminator) are hashed.
/// A null pointer is treated as a no-op and returns `seed` unchanged.
#[no_mangle]
pub extern "C" fn pcsx2_hash_combine_str(seed: u64, value: *const c_char) -> u64 {
    let mut s = seed;
    if value.is_null() {
        return s;
    }
    // Safety: caller guarantees `value` points to a valid, null-terminated
    // C string for the duration of this call. Null is handled above.
    let cstr = unsafe { CStr::from_ptr(value) };
    pcsx2_hash_combine_mix(&mut s, &[cstr.to_bytes()]);
    s
}