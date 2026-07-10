// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! `aligned_malloc` — aligned heap allocation shims.
//!
//! Rust port of PCSX2's C++ `common/AlignedMalloc.{h,cpp}`. The upstream
//! code provides portable wrappers around platform-specific aligned
//! allocators (`posix_memalign` on Unix, `_aligned_malloc` on Windows).
//! In Rust we get the same behaviour by forwarding to `libc` directly,
//! avoiding any extra runtime machinery.
//!
//! ## Safety
//!
//! All functions take raw pointers and call into `libc`, so every
//! public function is `unsafe`. Callers must follow the usual
//! allocator contract: pointers returned by [`aligned_malloc`] /
//! [`aligned_realloc`] must be released with [`aligned_free`], and
//! pointers passed to [`aligned_realloc`] / [`aligned_free`] must have
//! been allocated by the matching function (or be `null`).
//!
//! On Unix, alignment is asserted to be a power of two and a multiple
//! of `sizeof(void *)` (the requirements of `posix_memalign`); on
//! Windows, the runtime asserts alignment is below `0x10000`. Mirrors
//! the `pxAssert(align < 0x10000)` in the C++ source.
//!
//! ## FFI
//!
//! Three `extern "C"` functions are exported for the C++ side:
//! `pcsx2_aligned_malloc`, `pcsx2_aligned_realloc`,
//! `pcsx2_aligned_free`. They return / accept `*mut c_void` so callers
//! do not need to know the Rust pointer types.

use core::ffi::c_void;
use core::ptr;

// ---------------------------------------------------------------------------
// Pure-Rust surface
// ---------------------------------------------------------------------------

/// Allocate `size` bytes with the given `alignment`.
///
/// Returns a null pointer on allocation failure. The returned pointer
/// must be released with [`aligned_free`].
#[inline]
pub unsafe fn aligned_malloc(size: usize, alignment: usize) -> *mut u8 {
    debug_assert!(alignment != 0 && (alignment & (alignment - 1)) == 0,
        "alignment must be a power of two");

    let raw = aligned_malloc_raw(size, alignment);
    raw as *mut u8
}

/// Reallocate a previously-allocated aligned buffer.
///
/// If `ptr` is non-null, the first `min(old_size, size)` bytes are
/// copied to the new allocation and `ptr` is freed. The returned
/// pointer (which may be null on failure) must be released with
/// [`aligned_free`].
///
/// `old_size` is the size of the existing allocation; it is the
/// caller's responsibility to track this, mirroring the C++ signature.
pub unsafe fn aligned_realloc(
    ptr: *mut u8,
    size: usize,
    old_size: usize,
    alignment: usize,
) -> *mut u8 {
    let new_ptr = aligned_malloc(size, alignment);

    if !new_ptr.is_null() && !ptr.is_null() {
        let copy_len = if old_size < size { old_size } else { size };
        if copy_len > 0 {
            ptr::copy_nonoverlapping(ptr, new_ptr, copy_len);
        }
        aligned_free(ptr);
    }

    new_ptr
}

/// Free a buffer previously returned by [`aligned_malloc`] or
/// [`aligned_realloc`].
///
/// Safe to call with a null pointer (no-op).
#[inline]
pub unsafe fn aligned_free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    aligned_free_raw(ptr as *mut c_void);
}

// ---------------------------------------------------------------------------
// Platform-specific primitives
// ---------------------------------------------------------------------------

#[cfg(unix)]
#[inline]
unsafe fn aligned_malloc_raw(size: usize, alignment: usize) -> *mut c_void {
    // posix_memalign requires alignment to be a power of two AND a
    // multiple of sizeof(void *). Round up so callers passing
    // e.g. alignment=64 on a 32-bit target don't get EINVAL.
    let ptr_alignment = core::mem::size_of::<*mut c_void>();
    let effective_align = if alignment < ptr_alignment {
        ptr_alignment
    } else {
        alignment
    };

    let mut out: *mut c_void = ptr::null_mut();
    let rc = libc::posix_memalign(&mut out, effective_align, size);
    if rc != 0 {
        return ptr::null_mut();
    }
    out
}

#[cfg(windows)]
#[inline]
unsafe fn aligned_malloc_raw(size: usize, alignment: usize) -> *mut c_void {
    debug_assert!(alignment < 0x10000, "MSVCRT alignment must be < 0x10000");
    // _aligned_malloc is in MSVCRT (not re-exported by the `libc` crate);
    // declare it locally and call through the C ABI.
    extern "C" {
        fn _aligned_malloc(size: usize, alignment: usize) -> *mut c_void;
    }
    _aligned_malloc(size, alignment)
}

#[cfg(unix)]
#[inline]
unsafe fn aligned_free_raw(ptr: *mut c_void) {
    libc::free(ptr);
}

#[cfg(windows)]
#[inline]
unsafe fn aligned_free_raw(ptr: *mut c_void) {
    // _aligned_free is in MSVCRT (not re-exported by `libc`); declare it
    // locally.
    extern "C" {
        fn _aligned_free(ptr: *mut c_void);
    }
    _aligned_free(ptr);
}

// ---------------------------------------------------------------------------
// FFI surface
// ---------------------------------------------------------------------------

/// FFI wrapper for [`aligned_malloc`].
///
/// Returned pointer is null on failure. Must be released with
/// [`pcsx2_aligned_free`] (or a matching `_aligned_free` from MSVCRT
/// on Windows).
#[no_mangle]
pub extern "C" fn pcsx2_aligned_malloc(size: usize, alignment: usize) -> *mut c_void {
    unsafe { aligned_malloc(size, alignment) as *mut c_void }
}

/// FFI wrapper for [`aligned_realloc`].
///
/// `old_size` must be the size of the existing allocation so the
/// implementation can copy the live bytes over before freeing the old
/// buffer.
#[no_mangle]
pub extern "C" fn pcsx2_aligned_realloc(
    ptr: *mut c_void,
    size: usize,
    old_size: usize,
    alignment: usize,
) -> *mut c_void {
    unsafe {
        aligned_realloc(ptr as *mut u8, size, old_size, alignment) as *mut c_void
    }
}

/// FFI wrapper for [`aligned_free`]. No-op on null.
#[no_mangle]
pub extern "C" fn pcsx2_aligned_free(ptr: *mut c_void) {
    unsafe { aligned_free(ptr as *mut u8) }
}
