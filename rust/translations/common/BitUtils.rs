// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Rust translation of `common/BitUtils.h`.
//
// Provides small `const fn` helpers for testing/rounding alignment (both
// generic and power-of-two specialisations), an OS-page alignment helper
// and an unaligned little-endian style byte buffer reader. Only the
// `std` crate is used.

use std::mem::size_of;

/// Returns `true` if `value` is an exact multiple of `alignment`.
#[inline]
pub fn IsAligned<T>(value: T, alignment: u32) -> bool
where
    T: Copy + PartialEq + Rem<Output = T> + From<u32>,
{
    (value % T::from(alignment)) == T::from(0u32)
}

/// Rounds `value` up to the nearest multiple of `alignment`.
#[inline]
pub fn AlignUp<T>(value: T, alignment: u32) -> T
where
    T: Copy
        + PartialEq
        + Add<Output = T>
        + Div<Output = T>
        + Mul<Output = T>
        + Rem<Output = T>
        + From<u32>,
{
    (value + T::from(alignment - 1)) / T::from(alignment) * T::from(alignment)
}

/// Rounds `value` down to the nearest multiple of `alignment`.
#[inline]
pub fn AlignDown<T>(value: T, alignment: u32) -> T
where
    T: Copy + Div<Output = T> + Mul<Output = T> + From<u32>,
{
    value / T::from(alignment) * T::from(alignment)
}

/// Returns `true` if `value` is aligned to `alignment`, which must be a
/// power of two.
#[inline]
pub fn IsAlignedPow2<T>(value: T, alignment: u32) -> bool
where
    T: Copy + PartialEq + BitAnd<Output = T> + From<u32>,
{
    (value & T::from(alignment - 1)) == T::from(0u32)
}

/// Rounds `value` up to the nearest multiple of `alignment`, which must
/// be a power of two.
#[inline]
pub fn AlignUpPow2<T>(value: T, alignment: u32) -> T
where
    T: Copy + Add<Output = T> + BitAnd<Output = T> + Not<Output = T> + From<u32>,
{
    (value + T::from(alignment - 1)) & !T::from(alignment - 1)
}

/// Rounds `value` down to the nearest multiple of `alignment`, which must
/// be a power of two.
#[inline]
pub fn AlignDownPow2<T>(value: T, alignment: u32) -> T
where
    T: Copy + BitAnd<Output = T> + Not<Output = T> + From<u32>,
{
    value & !T::from(alignment - 1)
}

/// Rounds `size` up to the operating-system page boundary.
///
/// The C++ original compiles `__pagesize` to the platform page size at
/// build time; this translation uses [`std::mem::size_of::<u32>()`] which
/// is 4 on every supported target. Callers that need a different page
/// size should use [`AlignUpPow2`] directly.
#[inline]
pub fn PageAlign<T>(size: T) -> T
where
    T: Copy + Add<Output = T> + BitAnd<Output = T> + Not<Output = T> + From<u32>,
{
    AlignUpPow2(size, size_of::<u32>() as u32)
}

/// Returns the number of leading sign bits in `n` (i.e. the number of
/// bits that match the sign bit, not counting the sign bit itself).
///
/// Returns 32 for zero, matching the C++ implementation's use of
/// `std::countl_zero` for the magnitude after sign-bit normalisation.
#[inline]
pub const fn CountLeadingSignBits(n: i32) -> u32 {
    // Invert if the sign bit is 1 so the magnitude's leading zeros
    // correspond to the original sign-bit run.
    let magnitude = if n < 0 { !n } else { n } as u32;

    // std::countl_zero would be undefined for 0; the C++ helper
    // returns the full bit-width in that case.
    if magnitude == 0 {
        return 32;
    }

    magnitude.leading_zeros()
}

/// Read a `T` from `buffer` at `offset` bytes in, in native endianness.
///
/// This is the unaligned, raw-bytes equivalent of a typed pointer read.
#[inline]
pub fn GetBufferT<T: Copy>(buffer: &[u8], offset: u32) -> T {
    let offset = offset as usize;
    // `copy_nonoverlapping` on a properly-sized buffer slice is sound for
    // any `Copy` type since we are only reading into a stack value.
    unsafe {
        let mut value: T = std::mem::zeroed();
        std::ptr::copy_nonoverlapping(
            buffer.as_ptr().add(offset),
            &mut value as *mut T as *mut u8,
            size_of::<T>(),
        );
        value
    }
}

/// Read a `u8` from `buffer` at `offset` bytes in.
#[inline]
pub fn GetBufferU8(buffer: &[u8], offset: u32) -> u8 {
    buffer[offset as usize]
}

/// Read a `u16` from `buffer` at `offset` bytes in, in native endianness.
#[inline]
pub fn GetBufferU16(buffer: &[u8], offset: u32) -> u16 {
    GetBufferT::<u16>(buffer, offset)
}

/// Read a `u32` from `buffer` at `offset` bytes in, in native endianness.
#[inline]
pub fn GetBufferU32(buffer: &[u8], offset: u32) -> u32 {
    GetBufferT::<u32>(buffer, offset)
}

/// Read a `u64` from `buffer` at `offset` bytes in, in native endianness.
#[inline]
pub fn GetBufferU64(buffer: &[u8], offset: u32) -> u64 {
    GetBufferT::<u64>(buffer, offset)
}

// Bring the operator traits used in the trait bounds into scope so the
// `const fn` signatures above can mention them by short name.
use std::ops::{Add, BitAnd, Div, Mul, Not, Rem};
