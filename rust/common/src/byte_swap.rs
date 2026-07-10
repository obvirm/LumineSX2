// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Endianness byte-swap utilities.
//!
//! Mirrors PCSX2's `common/ByteSwap.h`. The C++ template `ByteSwap<T>`
//! dispatches to `_byteswap_ushort` / `_byteswap_ulong` / `_byteswap_uint64`
//! on MSVC, or `__builtin_bswap16/32/64` on GCC/Clang, and recurses into
//! `make_unsigned_t<T>` for signed integer types.
//!
//! The Rust port uses `std::intrinsics`-backed methods on the unsigned
//! primitives (`u16::swap_bytes`, `u32::swap_bytes`, ...) and re-exposes
//! a signed-integer API by transmuting through the unsigned representation.
//! A 128-bit variant is added for completeness (not in the original C++,
//! but trivially exposed through `u128::swap_bytes`).

/// Reverse the byte order of a `u16`.
///
/// Equivalent to `_byteswap_ushort` on MSVC and `__builtin_bswap16`
/// on GCC/Clang.
#[inline(always)]
pub fn byteswap16(val: u16) -> u16 {
    val.swap_bytes()
}

/// Reverse the byte order of a `u32`.
#[inline(always)]
pub fn byteswap32(val: u32) -> u32 {
    val.swap_bytes()
}

/// Reverse the byte order of a `u64`.
#[inline(always)]
pub fn byteswap64(val: u64) -> u64 {
    val.swap_bytes()
}

/// Reverse the byte order of a `u128`.
#[inline(always)]
pub fn byteswap128(val: u128) -> u128 {
    val.swap_bytes()
}

/// Reverse the byte order of an `i16`.
///
/// Signed counterpart to [`byteswap16`]; implemented by transmuting
/// through the unsigned representation, mirroring the C++ template's
/// `make_unsigned_t<T>` recursion.
#[inline(always)]
pub fn byteswap16_signed(val: i16) -> i16 {
    // Bit-for-bit identical transformation: `swap_bytes` operates on
    // the underlying bit pattern, so the unsigned and signed paths
    // produce the same bytes.
    val.swap_bytes()
}

/// Reverse the byte order of an `i32`.
#[inline(always)]
pub fn byteswap32_signed(val: i32) -> i32 {
    val.swap_bytes()
}

/// Reverse the byte order of an `i64`.
#[inline(always)]
pub fn byteswap64_signed(val: i64) -> i64 {
    val.swap_bytes()
}

/// Reverse the byte order of an `i128`.
#[inline(always)]
pub fn byteswap128_signed(val: i128) -> i128 {
    val.swap_bytes()
}

// ---------------------------------------------------------------------------
// FFI surface (consumed by C++ PCSX2 via cbindgen).
//
// The C++ side sees these as plain `extern "C"` functions; the mangling
// matches what `cbindgen.toml` is configured to emit.
// ---------------------------------------------------------------------------

/// FFI export: byte-swap a `uint16_t`.
#[no_mangle]
pub extern "C" fn pcsx2_byteswap16(val: u16) -> u16 {
    byteswap16(val)
}

/// FFI export: byte-swap a `uint32_t`.
#[no_mangle]
pub extern "C" fn pcsx2_byteswap32(val: u32) -> u32 {
    byteswap32(val)
}

/// FFI export: byte-swap a `uint64_t`.
#[no_mangle]
pub extern "C" fn pcsx2_byteswap64(val: u64) -> u64 {
    byteswap64(val)
}

/// FFI export: byte-swap a `__uint128_t` / `unsigned __int128`.
#[no_mangle]
pub extern "C" fn pcsx2_byteswap128(val: u128) -> u128 {
    byteswap128(val)
}

/// FFI export: byte-swap an `int16_t`.
#[no_mangle]
pub extern "C" fn pcsx2_byteswap16_signed(val: i16) -> i16 {
    byteswap16_signed(val)
}

/// FFI export: byte-swap an `int32_t`.
#[no_mangle]
pub extern "C" fn pcsx2_byteswap32_signed(val: i32) -> i32 {
    byteswap32_signed(val)
}

/// FFI export: byte-swap an `int64_t`.
#[no_mangle]
pub extern "C" fn pcsx2_byteswap64_signed(val: i64) -> i64 {
    byteswap64_signed(val)
}

/// FFI export: byte-swap a `__int128` / `__int128_t`.
#[no_mangle]
pub extern "C" fn pcsx2_byteswap128_signed(val: i128) -> i128 {
    byteswap128_signed(val)
}