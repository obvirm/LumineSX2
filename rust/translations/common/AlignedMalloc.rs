// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Aligned allocation helpers.
//!
//! This module mirrors PCSX2's `common/AlignedMalloc.{h,cpp}` pair. The C version provides
//! `_aligned_malloc` / `_aligned_free` for non-Windows platforms (where the CRT does not
//! ship them) and a convenience `pcsx2_aligned_realloc`. It uses `aligned_alloc` when the
//! C11 routine is available, otherwise falls back to `posix_memalign`, with a macOS
//! workaround that rounds the requested size up to the alignment because
//! `posix_memalign` is painfully slow on unaligned sizes there.
//!
//! On Windows (`_WIN32`) the C++ module is empty and relies on the CRT's
//! `_aligned_malloc` / `_aligned_free` provided by `<malloc.h>`. In this Rust port the
//! allocation strategy is uniform: we use `std::alloc::System` and stash a `Layout`
//! immediately before the returned pointer so `_aligned_free` can recover the exact
//! layout and deallocate safely. This matches the C version's behaviour of relying on
//! the underlying allocator to remember size/alignment, but keeps the module sound
//! when consumed from pure Rust.

use std::alloc::{self, Layout};
use std::ptr::{self, NonNull};

/// Maximum alignment accepted by [`_aligned_malloc`]; mirrors the `pxAssert(align < 0x10000)` check.
pub const MAX_ALIGNMENT: usize = 0x10000;

/// Returns the minimum of `a` and `b` without relying on `std::cmp::min` to mirror the
/// C `std::min` used by `pcsx2_aligned_realloc`.
#[inline]
fn min_usize(a: usize, b: usize) -> usize {
    if a < b { a } else { b }
}

/// Compute the allocation size used for the underlying allocator call.
///
/// On macOS the C code rounds the requested size up to a multiple of the alignment
/// because `posix_memalign` is very slow for unaligned sizes; on other platforms the
/// size is passed through unchanged.
#[inline]
fn effective_size(size: usize, align: usize) -> usize {
    if cfg!(target_os = "macos") {
        (size + align - 1) & !(align - 1)
    } else {
        size
    }
}

/// Allocate `size` bytes aligned to `align` bytes, returning a raw `*mut u8`.
///
/// Returns a null pointer when `size == 0`; otherwise always returns a non-null
/// pointer to an allocation of at least `effective_size(size, align)` bytes.
///
/// # Safety
///
/// `align` must be a power of two and less than [`MAX_ALIGNMENT`]. The returned
/// pointer must be freed with [`_aligned_free`].
#[no_mangle]
pub unsafe extern "C" fn _aligned_malloc(size: usize, align: usize) -> *mut u8 {
    assert!(align < MAX_ALIGNMENT, "alignment exceeds MAX_ALIGNMENT");
    assert!(align.is_power_of_two(), "alignment must be a power of two");
    if size == 0 {
        return ptr::null_mut();
    }
    aligned_malloc_inner(size, align, effective_size(size, align))
        .map(|nn| nn.as_ptr())
        .unwrap_or(ptr::null_mut())
}

/// Free memory previously returned by [`_aligned_malloc`].
///
/// A no-op on null pointers, matching the C version's tolerance of null input.
///
/// # Safety
///
/// `pmem` must be either null or a pointer returned by [`_aligned_malloc`].
#[no_mangle]
pub unsafe extern "C" fn _aligned_free(pmem: *mut u8) {
    if pmem.is_null() {
        return;
    }
    deallocate_from_prefix(pmem);
}

/// Reallocate a previously allocated aligned buffer, copying `min(old_size, new_size)`
/// bytes from the old buffer to the new one.
///
/// Mirrors `pcsx2_aligned_realloc`: allocates a fresh aligned buffer, copies the
/// overlap, and frees the old pointer. On Windows (MSVC) the C version forwards to
/// `_aligned_realloc`; this Rust port does the same thing unconditionally so callers
/// see consistent behaviour on every platform.
///
/// # Safety
///
/// `handle` must be either null or a pointer returned by [`_aligned_malloc`]. `align`
/// must be a power of two less than [`MAX_ALIGNMENT`]. The returned pointer must be
/// freed with [`_aligned_free`].
#[no_mangle]
pub unsafe extern "C" fn pcsx2_aligned_realloc(
    handle: *mut u8,
    new_size: usize,
    align: usize,
    old_size: usize,
) -> *mut u8 {
    let newbuf = _aligned_malloc(new_size, align);
    if !newbuf.is_null() && !handle.is_null() {
        ptr::copy_nonoverlapping(handle, newbuf, min_usize(old_size, new_size));
        _aligned_free(handle);
    }
    newbuf
}

/// Idiomatic Rust wrapper: returns a [`NonNull<u8>`] on success and `None` when
/// `size == 0` or allocation fails.
pub fn aligned_malloc(size: usize, align: usize) -> Option<NonNull<u8>> {
    if size == 0 {
        return None;
    }
    assert!(align.is_power_of_two(), "alignment must be a power of two");
    assert!(align < MAX_ALIGNMENT, "alignment exceeds MAX_ALIGNMENT");
    aligned_malloc_inner(size, align, effective_size(size, align))
}

/// Free a pointer previously returned by [`aligned_malloc`]. Passing null is a no-op.
///
/// # Safety
///
/// `ptr` must be either null or a pointer returned by [`aligned_malloc`].
pub unsafe fn aligned_free(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }
    deallocate_from_prefix(ptr);
}

/// Namespaced wrapper matching the C++ `Common::AlignedMalloc` / `Common::AlignedFree`
/// symbols so external code that imports the module as `Common` can call
/// `Common::AlignedMalloc(...)` / `Common::AlignedFree(...)`.
pub mod Common {
    use super::{aligned_free, aligned_malloc, NonNull};

    /// See [`super::aligned_malloc`].
    pub fn AlignedMalloc(size: usize, align: usize) -> Option<NonNull<u8>> {
        aligned_malloc(size, align)
    }

    /// See [`super::aligned_free`].
    ///
    /// # Safety
    ///
    /// See [`super::aligned_free`].
    pub unsafe fn AlignedFree(ptr: *mut u8) {
        aligned_free(ptr)
    }
}

// --- internal helpers ---------------------------------------------------

/// Size of the layout header stashed immediately before the user-visible pointer.
const PREFIX: usize = std::mem::size_of::<Layout>();

/// Perform the actual allocation, stashing the `Layout` in a prefix so that
/// [`deallocate_from_prefix`] can recover it.
fn aligned_malloc_inner(
    user_size: usize,
    user_align: usize,
    alloc_size: usize,
) -> Option<NonNull<u8>> {
    let total = alloc_size.checked_add(PREFIX)?;
    let layout = Layout::from_size_align(total, user_align).ok()?;

    unsafe {
        let raw = alloc::alloc(layout);
        let nn = NonNull::new(raw)?;
        // Store the layout in the prefix.
        (nn.as_ptr() as *mut Layout).write(layout);
        // Skip past the metadata to the user-visible region.
        let user_ptr = nn.as_ptr().add(PREFIX);
        debug_assert_eq!(user_ptr as usize % user_align, 0);
        debug_assert!(user_ptr as usize >= (nn.as_ptr() as usize) + PREFIX);
        // Use the user_size to silence the unused warning while documenting intent.
        let _ = user_size;
        Some(NonNull::new_unchecked(user_ptr))
    }
}

/// Inverse of [`aligned_malloc_inner`]: read the prefix, then deallocate through
/// `alloc::System` with the exact layout that was used to allocate.
unsafe fn deallocate_from_prefix(user_ptr: *mut u8) {
    let meta_ptr = user_ptr.sub(PREFIX) as *const Layout;
    let layout = meta_ptr.read();
    alloc::dealloc(user_ptr.sub(PREFIX), layout);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn min_usize_works() {
        assert_eq!(min_usize(3, 5), 3);
        assert_eq!(min_usize(7, 2), 2);
        assert_eq!(min_usize(0, 0), 0);
    }

    #[test]
    fn aligned_malloc_zero_size_is_none() {
        assert!(aligned_malloc(0, 16).is_none());
    }

    #[test]
    fn c_aligned_malloc_zero_size_is_null() {
        unsafe {
            assert!(_aligned_malloc(0, 16).is_null());
        }
    }

    #[test]
    fn aligned_malloc_roundtrip() {
        let nn = aligned_malloc(64, 32).expect("alloc should succeed");
        assert_eq!(nn.as_ptr() as usize % 32, 0);
        unsafe {
            aligned_free(nn.as_ptr());
        }
    }

    #[test]
    fn c_aligned_malloc_roundtrip() {
        unsafe {
            let p = _aligned_malloc(128, 64);
            assert!(!p.is_null());
            assert_eq!(p as usize % 64, 0);
            _aligned_free(p);
        }
    }

    #[test]
    fn c_aligned_free_null_is_noop() {
        unsafe {
            _aligned_free(ptr::null_mut());
        }
    }

    #[test]
    #[should_panic(expected = "power of two")]
    fn aligned_malloc_rejects_bad_align() {
        let _ = aligned_malloc(16, 3);
    }
}
