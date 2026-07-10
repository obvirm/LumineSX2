// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Basic atomic types — Rust translation of `common/Pcsx2Types.h`.
//!
//! The C++ header is a thin collection of fixed-width integer aliases plus
//! a pair of 128-bit types (`u128` / `s128`) that PCSX2 uses as a portable,
//! non-SSE 128-bit container. The Rust port re-uses Rust's native primitives
//! for everything:
//!
//! - `s8`..`s64` / `u8`..`u64` are aliases for the corresponding Rust
//!   integer primitives. Names that don't shadow a primitive are aliased
//!   directly; names that do (e.g. `u8`) use the fully-qualified
//!   `core::primitive::u8` path so the RHS is not shadowed by the
//!   alias being declared.
//! - `uptr` / `sptr` alias `usize` / `isize` directly.
//! - `uint` aliases `u32`, matching PCSX2's convention that `int` is
//!   32 bits (true on every platform PCSX2 supports).
//! - `u128` / `s128` are the native Rust 128-bit primitives. Their
//!   platform ABI is identical to C++'s `unsigned __int128` / `__int128`,
//!   so they cross the FFI boundary with no `#[repr(C)]` wrapper.
//!
//! # FFI surface
//!
//! The C++ side constructs 128-bit values via the static factories
//! `u128::From64` / `u128::From32` (and the corresponding `s128`
//! variants). The Rust equivalents are pure-Rust helpers exposed
//! through `#[no_mangle] pub extern "C"` shims below so the C++ core
//! can construct these values directly.
//!
//! Equality, the implicit narrowing conversions (`operator u32()` etc.),
//! and direct byte access (`_u64[0]` / `lo` / `hi`) all collapse to
//! ordinary `__int128` / `unsigned __int128` operations on the C++
//! side once the ABI matches, so they are not re-exported.

// ---------------------------------------------------------------------------
// Basic signed/unsigned integer width aliases.
//
// `pub type u8 = u8;` is a Rust error (the alias shadows its referent and
// the RHS becomes recursive), so names that collide with a primitive use
// the fully-qualified `core::primitive::u8` path. Non-colliding names
// (`s8`, `s16`, ..., `uptr`, `sptr`, `uint`) alias the primitive directly.
// ---------------------------------------------------------------------------

/// 8-bit signed integer. Alias for `i8` / C++ `int8_t`.
pub type s8 = i8;

/// 16-bit signed integer. Alias for `i16` / C++ `int16_t`.
pub type s16 = i16;

/// 32-bit signed integer. Alias for `i32` / C++ `int32_t`.
pub type s32 = i32;

/// 64-bit signed integer. Alias for `i64` / C++ `int64_t`.
pub type s64 = i64;

/// 8-bit unsigned integer. Alias for `u8` / C++ `uint8_t`.
pub type u8 = core::primitive::u8;

/// 16-bit unsigned integer. Alias for `u16` / C++ `uint16_t`.
pub type u16 = core::primitive::u16;

/// 32-bit unsigned integer. Alias for `u32` / C++ `uint32_t`.
pub type u32 = core::primitive::u32;

/// 64-bit unsigned integer. Alias for `u64` / C++ `uint64_t`.
pub type u64 = core::primitive::u64;

/// Unsigned pointer-width integer. Alias for `usize` / C++ `uintptr_t`.
pub type uptr = usize;

/// Signed pointer-width integer. Alias for `isize` / C++ `intptr_t`.
pub type sptr = isize;

/// Plain `unsigned int`. PCSX2's C++ code assumes `int` is 32-bit; so
/// does Rust (`u32` is guaranteed to be exactly 32 bits).
pub type uint = u32;

// ---------------------------------------------------------------------------
// 128-bit constructors — FFI surface.
//
// The C++ side calls `u128::From64(x)` / `u128::From32(x)` and
// `s128::From64(x)` / `s128::From32(x)` to produce 128-bit values
// from narrower sources. The Rust equivalents below are pure-Rust
// helpers; the `#[no_mangle] pub extern "C"` wrappers above them
// expose them to the C++ side via cbindgen.
//
// (Note: the C++ source labels the s32-taking s128 factory `From64`
// too — likely a copy/paste bug. The Rust port fixes that by giving
// the s32 overload its proper `From32` name.)
// ---------------------------------------------------------------------------

/// Zero-extend a 64-bit source through 128 bits. Equivalent to
/// `u128::From64` on the C++ side.
#[inline(always)]
pub fn u128_from64(src: u64) -> u128 {
    src as u128
}

/// Zero-extend a 32-bit source through 128 bits. Equivalent to
/// `u128::From32` on the C++ side.
#[inline(always)]
pub fn u128_from32(src: u32) -> u128 {
    src as u128
}

/// Sign-extend a 64-bit source through 128 bits. Equivalent to
/// `s128::From64` on the C++ side.
#[inline(always)]
pub fn s128_from64(src: i64) -> i128 {
    src as i128
}

/// Sign-extend a 32-bit source through 128 bits. Equivalent to
/// `s128::From32` on the C++ side.
#[inline(always)]
pub fn s128_from32(src: i32) -> i128 {
    src as i128
}

/// FFI export: zero-extend a `uint64_t` to `unsigned __int128`.
///
/// Mirrors the C++ `u128::From64` static factory.
#[no_mangle]
pub extern "C" fn pcsx2_u128_from64(src: u64) -> u128 {
    u128_from64(src)
}

/// FFI export: zero-extend a `uint32_t` to `unsigned __int128`.
///
/// Mirrors the C++ `u128::From32` static factory.
#[no_mangle]
pub extern "C" fn pcsx2_u128_from32(src: u32) -> u128 {
    u128_from32(src)
}

/// FFI export: sign-extend an `int64_t` to `__int128`.
///
/// Mirrors the C++ `s128::From64` static factory.
#[no_mangle]
pub extern "C" fn pcsx2_s128_from64(src: i64) -> i128 {
    s128_from64(src)
}

/// FFI export: sign-extend an `int32_t` to `__int128`.
///
/// Mirrors the C++ `s128::From32` static factory (which the C++ source
/// accidentally also labels `From64` — the Rust port fixes the name).
#[no_mangle]
pub extern "C" fn pcsx2_s128_from32(src: i32) -> i128 {
    s128_from32(src)
}