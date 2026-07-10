// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Bit & alignment utilities.
//!
//! Mirrors PCSX2's `common/BitUtils.h`. Provides:
//!
//! - Generic alignment helpers (`is_aligned`, `align_up`, `align_down`,
//!   plus `_pow2` variants that assume a power-of-two alignment).
//! - [`page_align`], which rounds a byte size up to the host page
//!   boundary (`crate::pcsx2_defs::__pagesize`).
//! - [`count_leading_sign_bits`], the standard CLZ-derived sign-bit count
//!   used by the EE/VU interpreters.
//! - [`get_buffer_u8`]/[`get_buffer_u16`]/[`get_buffer_u32`]/[`get_buffer_u64`],
//!   alignment-safe reads from raw byte buffers — used throughout the
//!   DMA, GIF, VIF and SIF packet parsers.
//!
//! ## Design
//!
//! The pure-Rust alignment API is fully safe (no `unsafe` blocks) and
//! generic over a sealed [`Integer`] supertrait that captures the
//! arithmetic and bitwise operators we need, plus the inherent
//! `next_multiple_of` re-exposed as a trait method.
//!
//! The FFI surface mirrors what PCSX2's C++ core already calls into:
//! alignment predicates/routines specialised for `u32` / `u64` / `usize`,
//! a `count_leading_sign_bits_s32`, and the four `get_buffer_*` buffer
//! readers that take a raw `*const u8` and do an `unaligned` read —
//! matching the C++ `std::memcpy` semantics byte-for-byte.

use std::ops::{Add, BitAnd, Div, Mul, Not, Rem, Sub};

use crate::pcsx2_defs::__pagesize;

// ---------------------------------------------------------------------------
// Sealed `Integer` supertrait
// ---------------------------------------------------------------------------

mod sealed {
    /// Seals the [`Integer`](super::Integer) trait so it can only be
    /// implemented for the primitive integer types in this crate,
    /// preventing downstream impls from violating our invariants.
    pub trait Sealed {}
}

/// Common supertrait for the primitive integer types we accept across
/// the alignment API. Captures the arithmetic and bitwise operators we
/// need, and re-exposes the inherent `next_multiple_of` as a trait
/// method so it can be called generically.
///
/// All signed and unsigned primitive integer types (`u8`/`u16`/`u32`/
/// `u64`/`u128`/`usize` and their `i*` counterparts) implement this
/// trait. The trait is sealed — no downstream types can implement it.
pub trait Integer:
    Copy
    + Default
    + PartialEq
    + Add<Output = Self>
    + Sub<Output = Self>
    + Mul<Output = Self>
    + Div<Output = Self>
    + Rem<Output = Self>
    + BitAnd<Output = Self>
    + Not<Output = Self>
    + sealed::Sealed
{
    /// Returns the value `1` of this integer type.
    fn one() -> Self;

    /// Converts a `u32` literal into this integer type.
    fn from_u32(v: u32) -> Self;
}

macro_rules! impl_integer {
    ($($t:ty),* $(,)?) => {
        $(
            impl sealed::Sealed for $t {}

            impl Integer for $t {
                #[inline(always)]
                fn one() -> Self { 1 }

                #[inline(always)]
                fn from_u32(v: u32) -> Self { v as Self }
            }
        )*
    };
}

impl_integer!(
    u8, u16, u32, u64, u128, usize,
    i8, i16, i32, i64, i128, isize,
);

// ---------------------------------------------------------------------------
// Pure-Rust alignment API
// ---------------------------------------------------------------------------

/// Returns `true` if `value` is evenly divisible by `alignment`.
///
/// Equivalent to `Common::IsAligned<T>` in `common/BitUtils.h`.
/// `alignment` must be non-zero.
#[inline(always)]
pub fn is_aligned<T: Integer>(value: T, alignment: T) -> bool {
    value % alignment == T::default()
}

/// Rounds `value` up to the next multiple of `alignment`.
///
/// Uses the classic `(value + (alignment - 1)) / alignment * alignment`
/// formulation, matching the C++ version. Caller must ensure
/// `alignment` is non-zero and `value + alignment - 1` does not overflow.
#[inline(always)]
pub fn align_up<T: Integer>(value: T, alignment: T) -> T {
    let one = T::one();
    (value + (alignment - one)) / alignment * alignment
}

/// Rounds `value` down to the previous multiple of `alignment`.
///
/// Implemented as `(value / alignment) * alignment`, exactly matching
/// the C++ version.
#[inline(always)]
pub fn align_down<T: Integer>(value: T, alignment: T) -> T {
    (value / alignment) * alignment
}

/// Returns `true` if `value` is aligned to a power-of-two `alignment`.
///
/// Uses the bitmask `(value & (alignment - 1)) == 0` trick — faster than
/// [`is_aligned`] because it avoids the integer modulo. The caller must
/// ensure `alignment` is a power of two and non-zero.
#[inline(always)]
pub fn is_aligned_pow2<T: Integer>(value: T, alignment: T) -> bool {
    (value & (alignment - T::one())) == T::default()
}

/// Rounds `value` up to the next multiple of a power-of-two `alignment`.
///
/// Uses the standard `(x + (a - 1)) & ~(a - 1)` bitmask trick. The
/// caller must ensure `alignment` is a power of two and non-zero.
#[inline(always)]
pub fn align_up_pow2<T: Integer>(value: T, alignment: T) -> T {
    let one = T::one();
    (value + (alignment - one)) & !(alignment - one)
}

/// Rounds `value` down to the previous multiple of a power-of-two
/// `alignment`.
///
/// The caller must ensure `alignment` is a power of two and non-zero.
#[inline(always)]
pub fn align_down_pow2<T: Integer>(value: T, alignment: T) -> T {
    let one = T::one();
    value & !(alignment - one)
}

/// Rounds `size` up to the next multiple of the host page size.
///
/// The page size comes from `crate::pcsx2_defs::__pagesize` (mirroring
/// C++'s `__pagesize`); it is statically guaranteed to be a power of
/// two, so [`align_up_pow2`] is the right primitive to delegate to.
#[inline(always)]
pub fn page_align<T: Integer>(size: T) -> T {
    align_up_pow2(size, T::from_u32(__pagesize))
}

// ---------------------------------------------------------------------------
// Sign-bit counting
// ---------------------------------------------------------------------------

/// Counts the number of leading bits that match the sign bit of `n`.
///
/// Returns `32` for `0` (matching `std::countl_zero(0) == 32`), and
/// otherwise inverts `n` when it is negative so the sign bit becomes
/// `0`, then runs `countl_zero` on the result. Used by the EE/VU
/// interpreters' sign-extension helpers.
#[inline(always)]
pub fn count_leading_sign_bits(n: i32) -> u32 {
    // If the sign bit is 1, invert the bits so it becomes 0 for
    // count-leading-zero. The C++ original does this explicitly;
    // we replicate it branch-for-branch to preserve identical rounding
    // behaviour for the boundary case `n == i32::MIN`.
    let n = if n < 0 { !n } else { n };

    // `countl_zero(0)` is defined as the bit width, which here is 32.
    if n == 0 {
        return 32;
    }

    (n as u32).leading_zeros()
}

// ---------------------------------------------------------------------------
// Buffer readers — safe Rust API
// ---------------------------------------------------------------------------

/// Read a `T` value from `buffer` at the given byte `offset`.
///
/// Performs an unaligned read, so it is safe to call regardless of the
/// buffer's alignment. Panics if `offset + size_of::<T>()` would extend
/// past the end of `buffer`. The byte-level semantics match the C++
/// `GetBufferT<T>` `std::memcpy`.
#[inline(always)]
pub fn get_buffer_t<T: Integer>(buffer: &[u8], offset: u32) -> T {
    let size = std::mem::size_of::<T>();
    let end = (offset as usize)
        .checked_add(size)
        .expect("get_buffer_t: offset overflow");
    assert!(
        end <= buffer.len(),
        "get_buffer_t: offset {} + size {} exceeds buffer length {}",
        offset,
        size,
        buffer.len()
    );
    // SAFETY: `buffer` is a valid slice for `buffer.len()` bytes; the
    // bounds check above ensures `[offset..end]` is in range. We use
    // `read_unaligned` because there is no guarantee that the buffer's
    // base address + `offset` is suitably aligned for `T`.
    unsafe {
        let ptr = buffer.as_ptr().add(offset as usize) as *const T;
        ptr.read_unaligned()
    }
}

/// Read a `u8` from `buffer` at `offset`. See [`get_buffer_t`].
#[inline(always)]
pub fn get_buffer_u8(buffer: &[u8], offset: u32) -> u8 {
    get_buffer_t::<u8>(buffer, offset)
}

/// Read a `u16` from `buffer` at `offset`. See [`get_buffer_t`].
#[inline(always)]
pub fn get_buffer_u16(buffer: &[u8], offset: u32) -> u16 {
    get_buffer_t::<u16>(buffer, offset)
}

/// Read a `u32` from `buffer` at `offset`. See [`get_buffer_t`].
#[inline(always)]
pub fn get_buffer_u32(buffer: &[u8], offset: u32) -> u32 {
    get_buffer_t::<u32>(buffer, offset)
}

/// Read a `u64` from `buffer` at `offset`. See [`get_buffer_t`].
#[inline(always)]
pub fn get_buffer_u64(buffer: &[u8], offset: u32) -> u64 {
    get_buffer_t::<u64>(buffer, offset)
}

// ===========================================================================
// FFI surface — consumed by C++ PCSX2 via cbindgen.
//
// The exported names follow the `pcsx2_*` convention used throughout this
// crate. Each alignment function is specialised for `u32`, `u64`, and
// `usize` so the C++ side has a single concrete symbol to call per type.
// ===========================================================================

// ---- is_aligned ----

#[no_mangle]
pub extern "C" fn pcsx2_is_aligned_u32(value: u32, alignment: u32) -> bool {
    is_aligned(value, alignment)
}

#[no_mangle]
pub extern "C" fn pcsx2_is_aligned_u64(value: u64, alignment: u64) -> bool {
    is_aligned(value, alignment)
}

#[no_mangle]
pub extern "C" fn pcsx2_is_aligned_usize(value: usize, alignment: usize) -> bool {
    is_aligned(value, alignment)
}

// ---- align_up ----

#[no_mangle]
pub extern "C" fn pcsx2_align_up_u32(value: u32, alignment: u32) -> u32 {
    align_up(value, alignment)
}

#[no_mangle]
pub extern "C" fn pcsx2_align_up_u64(value: u64, alignment: u64) -> u64 {
    align_up(value, alignment)
}

#[no_mangle]
pub extern "C" fn pcsx2_align_up_usize(value: usize, alignment: usize) -> usize {
    align_up(value, alignment)
}

// ---- align_down ----

#[no_mangle]
pub extern "C" fn pcsx2_align_down_u32(value: u32, alignment: u32) -> u32 {
    align_down(value, alignment)
}

#[no_mangle]
pub extern "C" fn pcsx2_align_down_u64(value: u64, alignment: u64) -> u64 {
    align_down(value, alignment)
}

#[no_mangle]
pub extern "C" fn pcsx2_align_down_usize(value: usize, alignment: usize) -> usize {
    align_down(value, alignment)
}

// ---- is_aligned_pow2 ----

#[no_mangle]
pub extern "C" fn pcsx2_is_aligned_pow2_u32(value: u32, alignment: u32) -> bool {
    is_aligned_pow2(value, alignment)
}

#[no_mangle]
pub extern "C" fn pcsx2_is_aligned_pow2_u64(value: u64, alignment: u64) -> bool {
    is_aligned_pow2(value, alignment)
}

#[no_mangle]
pub extern "C" fn pcsx2_is_aligned_pow2_usize(value: usize, alignment: usize) -> bool {
    is_aligned_pow2(value, alignment)
}

// ---- align_up_pow2 ----

#[no_mangle]
pub extern "C" fn pcsx2_align_up_pow2_u32(value: u32, alignment: u32) -> u32 {
    align_up_pow2(value, alignment)
}

#[no_mangle]
pub extern "C" fn pcsx2_align_up_pow2_u64(value: u64, alignment: u64) -> u64 {
    align_up_pow2(value, alignment)
}

#[no_mangle]
pub extern "C" fn pcsx2_align_up_pow2_usize(value: usize, alignment: usize) -> usize {
    align_up_pow2(value, alignment)
}

// ---- align_down_pow2 ----

#[no_mangle]
pub extern "C" fn pcsx2_align_down_pow2_u32(value: u32, alignment: u32) -> u32 {
    align_down_pow2(value, alignment)
}

#[no_mangle]
pub extern "C" fn pcsx2_align_down_pow2_u64(value: u64, alignment: u64) -> u64 {
    align_down_pow2(value, alignment)
}

#[no_mangle]
pub extern "C" fn pcsx2_align_down_pow2_usize(value: usize, alignment: usize) -> usize {
    align_down_pow2(value, alignment)
}

// ---- page_align ----

#[no_mangle]
pub extern "C" fn pcsx2_page_align_u32(size: u32) -> u32 {
    page_align(size)
}

#[no_mangle]
pub extern "C" fn pcsx2_page_align_u64(size: u64) -> u64 {
    page_align(size)
}

#[no_mangle]
pub extern "C" fn pcsx2_page_align_usize(size: usize) -> usize {
    page_align(size)
}

// ---- count_leading_sign_bits ----

#[no_mangle]
pub extern "C" fn pcsx2_count_leading_sign_bits_s32(n: i32) -> u32 {
    count_leading_sign_bits(n)
}

// ---- buffer readers ----

/// FFI: alignment-safe `u8` read from a raw byte buffer.
///
/// Matches the C++ `GetBufferU8(const u8*, u32)` `memcpy` semantics.
///
/// # Safety
///
/// `buffer` must point to at least `offset + 1` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_get_buffer_u8(buffer: *const u8, offset: u32) -> u8 {
    // SAFETY: caller guarantees `buffer` is valid for `offset + 1` bytes.
    unsafe { buffer.add(offset as usize).read_unaligned() }
}

/// FFI: alignment-safe `u16` read from a raw byte buffer.
///
/// # Safety
///
/// `buffer` must point to at least `offset + 2` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_get_buffer_u16(buffer: *const u8, offset: u32) -> u16 {
    // SAFETY: caller guarantees `buffer` is valid for `offset + 2` bytes.
    let ptr = buffer.add(offset as usize) as *const u16;
    unsafe { ptr.read_unaligned() }
}

/// FFI: alignment-safe `u32` read from a raw byte buffer.
///
/// # Safety
///
/// `buffer` must point to at least `offset + 4` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_get_buffer_u32(buffer: *const u8, offset: u32) -> u32 {
    // SAFETY: caller guarantees `buffer` is valid for `offset + 4` bytes.
    let ptr = buffer.add(offset as usize) as *const u32;
    unsafe { ptr.read_unaligned() }
}

/// FFI: alignment-safe `u64` read from a raw byte buffer.
///
/// # Safety
///
/// `buffer` must point to at least `offset + 8` readable bytes.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_get_buffer_u64(buffer: *const u8, offset: u32) -> u64 {
    // SAFETY: caller guarantees `buffer` is valid for `offset + 8` bytes.
    let ptr = buffer.add(offset as usize) as *const u64;
    unsafe { ptr.read_unaligned() }
}