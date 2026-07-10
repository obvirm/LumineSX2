// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Pure-Rust port of `common/WrappedMemCopy.h`.
// Provides ring-buffer (wrap-around) `memcpy` primitives used by the GS
// thread when exchanging EE data with a circular DMA buffer. The original
// C++ header ships two `__ri` static helpers — one for a wrapping
// destination, one for a wrapping source — operating on 16-byte (`u128`)
// units. The Rust port preserves the algorithm verbatim and exposes both
// idiomatic slice-based safe entry points and the corresponding C-ABI FFI
// exports the GS-side shim expects.
//
// Notes on alignment: the C++ source assumes `u128` (16-byte) alignment
// for both buffers, which on x86/ARM hosts is the natural alignment of a
// 128-bit integer. The Rust port inherits that assumption through the use
// of `u128` slices, which carry the same alignment requirements.
//
// Notes on length: `len` here is measured in `u128` elements, *not* in
// bytes. The original C++ multiplies by 16 only inside the `memcpy` call;
// the count passed across the boundary is always an element count.

#![allow(clippy::missing_safety_doc)]

// ============================================================================
// Public safe API — slice-based
// ============================================================================

/// Copy `src` into a wrapping ring-buffer destination.
///
/// `src.len()` elements are copied into `dest_base` starting at index
/// `*dest_start`. When the write crosses the end of `dest_base` the copy
/// wraps around to the beginning. On return, `*dest_start` is updated to
/// the index immediately after the last written element, expressed in the
/// same element units as the slice (i.e. `% dest_base.len()` after a wrap).
///
/// `src.len()` must be `<= dest_base.len()`; larger inputs would overflow
/// the ring and are undefined behaviour both in the C++ original and here
/// (the safe `copy_from_slice` will panic before that happens, which is
/// strictly safer than the C++ side).
///
/// # Example
///
/// ```
/// let mut ring = [0u128; 8];
/// let mut pos = 6u32;          // near the end
/// let payload = [1, 2, 3, 4];
/// crate::wrapped_mem_copy::memcpy_wrapped_dest(&payload, &mut ring, &mut pos);
/// assert_eq!(pos, 2);          // wrapped to (6 + 4) % 8
/// // ring now contains src wrapped across the seam at index 8.
/// ```
#[inline]
pub fn memcpy_wrapped_dest(src: &[u128], dest_base: &mut [u128], dest_start: &mut u32) {
    let len = src.len() as u32;
    let dest_size = dest_base.len() as u32;
    let start = *dest_start;
    let endpos = start + len;

    if endpos < dest_size {
        // Fast path: the entire write fits before the wrap point.
        dest_base[start as usize..(start + len) as usize].copy_from_slice(src);
        *dest_start = endpos;
    } else {
        // Wrap path: fill the tail, then continue from index 0.
        let first_copy_len = dest_size - start;
        let first = first_copy_len as usize;
        dest_base[start as usize..dest_size as usize].copy_from_slice(&src[..first]);
        // After the wrap, `dest_start` is the remainder of `endpos` modulo
        // `dest_size` — exactly the number of elements that still need to
        // be written from the second half of `src`.
        *dest_start = endpos % dest_size;
        let second = *dest_start as usize;
        dest_base[..second].copy_from_slice(&src[first..first + second]);
    }
}

/// Copy from a wrapping ring-buffer source into a contiguous destination.
///
/// `dest.len()` elements are read from `src_base` starting at index
/// `*src_start`, wrapping around to the beginning when the read crosses
/// the end of `src_base`. On return, `*src_start` is updated to the index
/// immediately after the last read element.
///
/// `dest.len()` must be `<= src_base.len()` for the same reasons documented
/// on [`memcpy_wrapped_dest`].
#[inline]
pub fn memcpy_wrapped_src(src_base: &[u128], src_start: &mut u32, dest: &mut [u128]) {
    let len = dest.len() as u32;
    let src_size = src_base.len() as u32;
    let start = *src_start;
    let endpos = start + len;

    if endpos < src_size {
        // Fast path: the entire read fits before the wrap point.
        dest.copy_from_slice(&src_base[start as usize..(start + len) as usize]);
        *src_start = endpos;
    } else {
        // Wrap path: drain the tail, then continue from index 0.
        let first_copy_len = src_size - start;
        let first = first_copy_len as usize;
        dest[..first].copy_from_slice(&src_base[start as usize..src_size as usize]);
        *src_start = endpos % src_size;
        let second = *src_start as usize;
        dest[first..first + second].copy_from_slice(&src_base[..second]);
    }
}

// ============================================================================
// FFI surface — C ABI matching the GS thread's `WrappedMemCopy.h` callers
// ============================================================================
//
// The FFI shims take raw pointers (the only place `unsafe` is permitted
// per the file's conventions) and use `std::ptr::copy_nonoverlapping` for
// the actual element movement, matching the `memcpy`-on-`u128` semantics
// of the original C++ code. Both helpers tolerate `len == 0` (no-op) and
// defensive null pointers — the latter matches the style of the other
// FFI exports in this crate (see e.g. `md5_digest::pcsx2_md5_*`).

/// FFI wrapper for [`memcpy_wrapped_dest`].
///
/// Mirrors the C++ signature:
/// ```c
/// void pcsx2_memcpy_wrapped_dest(const u128* src,
///                                u128* destBase,
///                                uint* destStart,
///                                uint destSize,
///                                uint len);
/// ```
///
/// # Safety
///
/// - `src` must point to `len` readable `u128` elements.
/// - `dest_base` must point to `dest_size` writable `u128` elements.
/// - `dest_start` must point to a writable `u32`.
/// - All three pointers must be non-null for any non-zero `len`; null
///   pointers are silently ignored.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_memcpy_wrapped_dest(
    src: *const u128,
    dest_base: *mut u128,
    dest_start: *mut u32,
    dest_size: u32,
    len: u32,
) {
    if src.is_null() || dest_base.is_null() || dest_start.is_null() || len == 0 {
        return;
    }
    // Safety: pointers are non-null and bounds are given by the caller.
    let start = unsafe { *dest_start };
    let endpos = start + len;
    unsafe {
        if endpos < dest_size {
            // Fast path: single contiguous copy, no wrap.
            std::ptr::copy_nonoverlapping(src, dest_base.add(start as usize), len as usize);
            *dest_start = endpos;
        } else {
            // Wrap path: fill the tail of `dest_base`, then continue from 0.
            let first_copy_len = dest_size - start;
            std::ptr::copy_nonoverlapping(
                src,
                dest_base.add(start as usize),
                first_copy_len as usize,
            );
            // After wrapping, the new `dest_start` is the remainder of
            // `endpos` modulo `dest_size` — exactly the number of elements
            // still to write from the second half of `src`.
            let new_start = endpos % dest_size;
            *dest_start = new_start;
            std::ptr::copy_nonoverlapping(
                src.add(first_copy_len as usize),
                dest_base,
                new_start as usize,
            );
        }
    }
}

/// FFI wrapper for [`memcpy_wrapped_src`].
///
/// Mirrors the C++ signature:
/// ```c
/// void pcsx2_memcpy_wrapped_src(const u128* srcBase,
///                               uint* srcStart,
///                               uint srcSize,
///                               u128* dest,
///                               uint len);
/// ```
///
/// # Safety
///
/// - `src_base` must point to `src_size` readable `u128` elements.
/// - `src_start` must point to a writable `u32`.
/// - `dest` must point to `len` writable `u128` elements.
/// - All three pointers must be non-null for any non-zero `len`; null
///   pointers are silently ignored.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_memcpy_wrapped_src(
    src_base: *const u128,
    src_start: *mut u32,
    src_size: u32,
    dest: *mut u128,
    len: u32,
) {
    if src_base.is_null() || src_start.is_null() || dest.is_null() || len == 0 {
        return;
    }
    // Safety: pointers are non-null and bounds are given by the caller.
    let start = unsafe { *src_start };
    let endpos = start + len;
    unsafe {
        if endpos < src_size {
            // Fast path: single contiguous copy, no wrap.
            std::ptr::copy_nonoverlapping(
                src_base.add(start as usize),
                dest,
                len as usize,
            );
            *src_start = endpos;
        } else {
            // Wrap path: drain the tail of `src_base`, then continue from 0.
            let first_copy_len = src_size - start;
            std::ptr::copy_nonoverlapping(
                src_base.add(start as usize),
                dest,
                first_copy_len as usize,
            );
            let new_start = endpos % src_size;
            *src_start = new_start;
            std::ptr::copy_nonoverlapping(
                src_base,
                dest.add(first_copy_len as usize),
                new_start as usize,
            );
        }
    }
}