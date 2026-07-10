// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Rust translation of `common/WrappedMemCopy.h`.
//
// DMA helpers that move 16-byte (`u128`) units between a contiguous
// scratch buffer and a circular ("wrapped") ring buffer, advancing the
// ring's read/write cursor on the way. The EE's DMA engine hands the
// emulator a pair of ring buffers; these primitives are the lowest level
// of the transfer path and are written to be inlinable so the optimiser
// can fold them into the surrounding DMA loop.

use core::ptr;

/// Number of bytes in one DMA transfer unit. Matches the C++ `u128`
/// granularity used by `MemCopy_WrappedDest` / `MemCopy_WrappedSrc`.
pub const UNIT_BYTES: usize = 16;

/// Copy `len` 16-byte units from the contiguous source at `src` into the
/// circular destination ring `dest_base` (of `dest_size` units) starting
/// at the cursor `dest_start`. On return the cursor is advanced by `len`,
/// wrapping at `dest_size`.
///
/// Equivalent to `MemCopy_WrappedDest` in `common/WrappedMemCopy.h`.
///
/// # Safety
///
/// * `src` must point to `len * UNIT_BYTES` readable bytes.
/// * `dest_base` must point to `dest_size * UNIT_BYTES` writable bytes.
/// * The two ranges must not overlap (matches the C++ `__restrict`
///   semantics implied by the `__ri` annotation on the original).
/// * `dest_start` must be strictly less than `dest_size`, and
///   `dest_size` must be non-zero.
#[inline]
pub unsafe fn mem_copy_wrapped_dest(
    src: *const u128,
    dest_base: *mut u128,
    dest_start: &mut usize,
    dest_size: usize,
    len: usize,
) {
    debug_assert!(dest_size > 0, "dest_size must be non-zero");
    debug_assert!(*dest_start < dest_size, "dest_start must be < dest_size");

    let start = *dest_start;
    let endpos = start + len;
    if endpos < dest_size {
        // Fast path: the entire transfer fits without wrapping.
        ptr::copy_nonoverlapping(src, dest_base.add(start), len);
        *dest_start = endpos;
    } else {
        // Slow path: copy the tail of the ring, then wrap around and
        // copy the head.
        let first_copy_len = dest_size - start;
        ptr::copy_nonoverlapping(src, dest_base.add(start), first_copy_len);
        *dest_start = endpos % dest_size;
        ptr::copy_nonoverlapping(src.add(first_copy_len), dest_base, *dest_start);
    }
}

/// Copy `len` 16-byte units from the circular source ring `src_base` (of
/// `src_size` units) starting at the cursor `src_start` into the
/// contiguous destination at `dest`. On return the cursor is advanced by
/// `len`, wrapping at `src_size`.
///
/// Equivalent to `MemCopy_WrappedSrc` in `common/WrappedMemCopy.h`.
///
/// # Safety
///
/// * `src_base` must point to `src_size * UNIT_BYTES` readable bytes.
/// * `dest` must point to `len * UNIT_BYTES` writable bytes.
/// * The two ranges must not overlap.
/// * `src_start` must be strictly less than `src_size`, and `src_size`
///   must be non-zero.
#[inline]
pub unsafe fn mem_copy_wrapped_src(
    src_base: *const u128,
    src_start: &mut usize,
    src_size: usize,
    dest: *mut u128,
    len: usize,
) {
    debug_assert!(src_size > 0, "src_size must be non-zero");
    debug_assert!(*src_start < src_size, "src_start must be < src_size");

    let start = *src_start;
    let endpos = start + len;
    if endpos < src_size {
        // Fast path: the entire read fits without wrapping.
        ptr::copy_nonoverlapping(src_base.add(start), dest, len);
        *src_start = endpos;
    } else {
        // Slow path: copy the tail of the ring, then wrap around and
        // copy the head into the remaining destination slots.
        let first_copy_len = src_size - start;
        ptr::copy_nonoverlapping(src_base.add(start), dest, first_copy_len);
        *src_start = endpos % src_size;
        ptr::copy_nonoverlapping(src_base, dest.add(first_copy_len), *src_start);
    }
}

/// Byte-level DMA copy. Thin unsafe wrapper around
/// [`ptr::copy_nonoverlapping`] for callers that already work in raw
/// byte pointers and lengths rather than `u128` units. The C++ source
/// does not expose this signature directly, but the public translation
/// surface needs it so other Rust modules can hand a `wrapped_memcpy`
/// straight into DMA plumbing without re-deriving the byte length from
/// a unit count.
///
/// # Safety
///
/// * `src` must be valid for reads of `len` bytes.
/// * `dst` must be valid for writes of `len` bytes.
/// * The two ranges must not overlap. Use [`ptr::copy`] if they may.
#[inline]
pub unsafe fn wrapped_memcpy(dst: *mut u8, src: *const u8, len: usize) {
    ptr::copy_nonoverlapping(src, dst, len);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrapped_dest_no_wrap() {
        let src = [1u128, 2, 3, 4];
        let mut ring = [0u128; 8];
        let mut start = 2usize;
        unsafe {
            mem_copy_wrapped_dest(src.as_ptr(), ring.as_mut_ptr(), &mut start, ring.len(), src.len());
        }
        assert_eq!(start, 6);
        assert_eq!(&ring[2..6], &src[..]);
    }

    #[test]
    fn wrapped_dest_wraps() {
        let src = [10u128, 20, 30, 40, 50, 60, 70];
        let mut ring = [0u128; 5];
        let mut start = 3usize;
        unsafe {
            mem_copy_wrapped_dest(src.as_ptr(), ring.as_mut_ptr(), &mut start, ring.len(), src.len());
        }
        // 3 + 7 = 10, mod 5 = 0. First copy length = 5 - 3 = 2.
        assert_eq!(start, 0);
        assert_eq!(ring[3], 10);
        assert_eq!(ring[4], 20);
        assert_eq!(ring[0], 30);
        assert_eq!(ring[1], 40);
        assert_eq!(ring[2], 50);
    }

    #[test]
    fn wrapped_src_no_wrap() {
        let ring = [1u128, 2, 3, 4, 5, 6];
        let mut dest = [0u128; 3];
        let mut start = 1usize;
        unsafe {
            mem_copy_wrapped_src(ring.as_ptr(), &mut start, ring.len(), dest.as_mut_ptr(), dest.len());
        }
        assert_eq!(start, 4);
        assert_eq!(dest, [2, 3, 4]);
    }

    #[test]
    fn wrapped_src_wraps() {
        let ring = [1u128, 2, 3, 4, 5];
        let mut dest = [0u128; 4];
        let mut start = 3usize;
        unsafe {
            mem_copy_wrapped_src(ring.as_ptr(), &mut start, ring.len(), dest.as_mut_ptr(), dest.len());
        }
        // 3 + 4 = 7, mod 5 = 2. First copy length = 5 - 3 = 2.
        assert_eq!(start, 2);
        assert_eq!(dest, [4, 5, 1, 2]);
    }

    #[test]
    fn byte_memcpy_roundtrip() {
        let src = [0xABu8, 0xCD, 0xEF, 0x12];
        let mut dst = [0u8; 4];
        unsafe {
            wrapped_memcpy(dst.as_mut_ptr(), src.as_ptr(), dst.len());
        }
        assert_eq!(dst, src);
    }
}
