// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Rust translation of `common/ByteSwap.h`.
//
// Provides byte-swap helpers for the standard unsigned integer widths plus
// thin wrappers that operate on raw `u8` byte slices. The primitive
// implementations forward to the standard library's `to_le`/`to_be`/
// `from_le`/`from_be` and `swap_bytes` intrinsics, which the optimiser is
// expected to lower to architecture-native `bswap` instructions on x86 and
// ARM.

use std::mem::{size_of, transmute_copy};

/// Byte-swap a `u16` (big <-> little endian conversion).
#[inline]
pub const fn BSwap16(value: u16) -> u16 {
    value.swap_bytes()
}

/// Byte-swap a `u32` (big <-> little endian conversion).
#[inline]
pub const fn BSwap32(value: u32) -> u32 {
    value.swap_bytes()
}

/// Byte-swap a `u64` (big <-> little endian conversion).
#[inline]
pub const fn BSwap64(value: u64) -> u64 {
    value.swap_bytes()
}

/// Byte-swap a value of any supported integer width. Signed integers are
/// bit-cast to their unsigned counterpart, swapped, and then bit-cast back
/// so the result is well-defined for negative inputs as well.
#[inline]
pub fn ByteSwap<T: ByteSwapable>(value: T) -> T {
    value.byte_swap()
}

/// Marker trait for integer types that can be byte-swapped.
pub trait ByteSwapable: Copy {
    /// Perform the byte swap.
    fn byte_swap(self) -> Self;
}

impl ByteSwapable for u8 {
    #[inline]
    fn byte_swap(self) -> Self {
        // A single byte is its own swap.
        self
    }
}

impl ByteSwapable for u16 {
    #[inline]
    fn byte_swap(self) -> Self {
        BSwap16(self)
    }
}

impl ByteSwapable for u32 {
    #[inline]
    fn byte_swap(self) -> Self {
        BSwap32(self)
    }
}

impl ByteSwapable for u64 {
    #[inline]
    fn byte_swap(self) -> Self {
        BSwap64(self)
    }
}

impl ByteSwapable for i16 {
    #[inline]
    fn byte_swap(self) -> Self {
        BSwap16(self as u16) as i16
    }
}

impl ByteSwapable for i32 {
    #[inline]
    fn byte_swap(self) -> Self {
        BSwap32(self as u32) as i32
    }
}

impl ByteSwapable for i64 {
    #[inline]
    fn byte_swap(self) -> Self {
        BSwap64(self as u64) as i64
    }
}

/// Read a little-endian `u16` from a `u8` buffer and byte-swap it to host
/// order. Panics if `bytes.len() < 2`.
#[inline]
pub const fn ByteSwap16Bytes(bytes: &[u8]) -> u16 {
    assert!(bytes.len() >= 2, "ByteSwap16Bytes requires at least 2 bytes");
    let raw = u16::from_le_bytes([bytes[0], bytes[1]]);
    raw.swap_bytes()
}

/// Read a little-endian `u32` from a `u8` buffer and byte-swap it to host
/// order. Panics if `bytes.len() < 4`.
#[inline]
pub const fn ByteSwap32Bytes(bytes: &[u8]) -> u32 {
    assert!(bytes.len() >= 4, "ByteSwap32Bytes requires at least 4 bytes");
    let arr = [bytes[0], bytes[1], bytes[2], bytes[3]];
    let raw = u32::from_le_bytes(arr);
    raw.swap_bytes()
}

/// Read a little-endian `u64` from a `u8` buffer and byte-swap it to host
/// order. Panics if `bytes.len() < 8`.
#[inline]
pub const fn ByteSwap64Bytes(bytes: &[u8]) -> u64 {
    assert!(bytes.len() >= 8, "ByteSwap64Bytes requires at least 8 bytes");
    let arr = [bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7]];
    let raw = u64::from_le_bytes(arr);
    raw.swap_bytes()
}

/// Read a little-endian integer of any supported width from a `u8` buffer
/// and return its byte-swapped value. The element type is inferred from the
/// call site; panics if the buffer is too small to hold one element.
///
/// `transmute_copy` is used to reinterpret the swapped bit pattern as `T`
/// after a width-tagged read; the source and destination are both `Copy` so
/// this is safe.
#[inline]
pub fn ByteSwapBytes<T: ByteSwapable>(bytes: &[u8]) -> T {
    let n = size_of::<T>();
    assert!(bytes.len() >= n, "ByteSwapBytes requires at least {} bytes", n);

    let mut buf = [0u8; 8];
    buf[..n].copy_from_slice(&bytes[..n]);

    // Perform the swap in the widest supported unsigned width, then
    // bit-cast back to the caller's type.
    let swapped: u64 = match n {
        1 => buf[0] as u64,
        2 => (u16::from_le_bytes([buf[0], buf[1]])).swap_bytes() as u64,
        4 => (u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]])).swap_bytes() as u64,
        8 => u64::from_le_bytes(buf).swap_bytes(),
        _ => unreachable!("ByteSwapBytes: unsupported width {}", n),
    };

    // SAFETY: `T` and `u64` are both plain `Copy` integers of a power-of-two
    // byte width covered by the match above; the bit pattern of `swapped`
    // is a valid representation of `T` after the swap.
    unsafe { transmute_copy::<u64, T>(&swapped) }
}
