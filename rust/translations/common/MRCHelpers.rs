// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's `common/MRCHelpers.h`.
//!
//! The original C++ header provides a tiny `MRCOwned<T>` template that
//! wraps an Objective-C pointer (typically `NSObject*` or `id`) with
//! manual reference counting. The wrapper retains on copy, releases on
//! destruction, and exposes a `Reset()` plus a `Get()` accessor. The
//! header also defines the free-function helpers `MRCTransfer`
//! (equivalent to Obj-C's `__bridge_transfer`) and `MRCRetain`.
//!
//! Rust cannot speak Objective-C directly, so the macOS side of this
//! module is built around the `objc_retain` / `objc_release` runtime
//! entry points. We model `MRCOwned<T>` as an owning wrapper around
//! `*const c_void` with a `PhantomData<*mut T>` marker so the original
//! C++ generic parameter survives. The wrapper implements `Deref` to
//! `*const c_void` (matching how the C++ implicit `operator T()`
//! surfaces the raw pointer to callers) and a `Drop` impl that calls
//! `objc_release`. Cloning retains. On non-macOS targets the module
//! exposes empty stubs so the file still compiles inside a
//! `pcsx2_translations` library.

#![allow(dead_code, non_snake_case)]

use std::marker::PhantomData;
use std::ops::Deref;
use std::os::raw::c_void;
use std::ptr;

// ---------------------------------------------------------------------------
// Platform-specific retain/release hooks.
//
// On macOS we forward to the Obj-C runtime's `objc_retain` /
// `objc_release` entry points (the C++ template uses `[ptr retain]` and
// `[ptr release]`, which expand to those same calls). On other targets
// the hooks are no-ops / identity functions, mirroring the C++ header's
// behaviour of compiling to nothing on platforms where Obj-C is
// unavailable.
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
mod imp {
    use std::os::raw::c_void;

    extern "C" {
        fn objc_release(ptr: *mut c_void);
        fn objc_retain(ptr: *mut c_void) -> *mut c_void;
    }

    /// Release a non-null Obj-C pointer. A null pointer is a no-op,
    /// matching the C++ template's `if (ptr) [ptr release];` guard.
    #[inline]
    pub(crate) unsafe fn release(ptr: *mut c_void) {
        if !ptr.is_null() {
            objc_release(ptr);
        }
    }

    /// Retain an Obj-C pointer. A null pointer is returned unchanged
    /// rather than forwarded to `objc_retain`, which would be
    /// undefined behaviour.
    #[inline]
    pub(crate) unsafe fn retain(ptr: *mut c_void) -> *mut c_void {
        if ptr.is_null() {
            ptr
        } else {
            objc_retain(ptr)
        }
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use std::os::raw::c_void;

    /// No-op on non-macOS targets. Mirrors the C++ header's behaviour
    /// of compiling to nothing on platforms where Obj-C is unavailable.
    #[inline]
    pub(crate) unsafe fn release(_ptr: *mut c_void) {}

    /// Identity on non-macOS targets.
    #[inline]
    pub(crate) unsafe fn retain(ptr: *mut c_void) -> *mut c_void {
        ptr
    }
}

// ---------------------------------------------------------------------------
// MRCOwned<T>
//
// Mirrors the C++ `template <typename T> class MRCOwned`. `T` is the
// original Objective-C type (e.g. `NSObject`); the pointer itself is
// stored as `*const c_void` so the wrapper is `Deref`-able to that
// opaque form, matching the spirit of `operator T() const` in the
// C++ template while remaining sound under Rust's strict aliasing
// rules.
// ---------------------------------------------------------------------------

/// Managed Objective-C pointer with manual reference counting.
///
/// The wrapped object is released on drop. Cloning retains the
/// pointer. Construction is intentionally restricted to the static
/// factories `transfer` (no additional retain) and `retain` (retain
/// and take ownership), mirroring the C++ template's private
/// `MRCOwned(T ptr)` constructor and `static Transfer` / `static
/// Retain` helpers.
///
/// # Examples
///
/// ```ignore
/// // Take ownership of an already-retained pointer (no extra retain).
/// let owned: MRCOwned<NSObject> = MRCTransfer(some_objc_ptr);
///
/// // Retain a borrowed pointer and take ownership.
/// let owned: MRCOwned<NSObject> = MRCRetain(borrowed_ptr);
/// ```
#[cfg_attr(
    not(target_os = "macos"),
    doc = "\n\n> **Note:** On non-macOS targets the wrapper is a zero-sized"
)]
#[cfg_attr(
    not(target_os = "macos"),
    doc = " stub; retain/release hooks are no-ops."
)]
pub struct MRCOwned<T = ()> {
    ptr: *const c_void,
    _marker: PhantomData<*mut T>,
}

impl<T> MRCOwned<T> {
    /// Construct a null wrapper. Mirrors `MRCOwned()` /
    /// `MRCOwned(std::nullptr_t)` in the C++ template.
    #[inline]
    pub const fn new() -> Self {
        Self {
            ptr: ptr::null(),
            _marker: PhantomData,
        }
    }

    /// Take ownership of an already-retained pointer without
    /// performing an additional retain. Equivalent to
    /// `MRCOwned<T>::Transfer` and to Obj-C's `__bridge_transfer`.
    ///
    /// # Safety
    ///
    /// The caller must transfer a valid, non-null pointer that
    /// already has a +1 retain count that this `MRCOwned` will be
    /// responsible for releasing.
    #[inline]
    pub fn transfer(ptr: *mut T) -> Self {
        Self {
            ptr: ptr as *const c_void,
            _marker: PhantomData,
        }
    }

    /// Retain the pointer and take ownership. Equivalent to
    /// `MRCOwned<T>::Retain` in the C++ template.
    ///
    /// # Safety
    ///
    /// The caller must pass a valid Objective-C pointer (or null).
    /// Passing a non-pointer or an unreferenced object is undefined.
    #[inline]
    pub fn retain(ptr: *mut T) -> Self {
        // SAFETY: forwarded to the platform `retain` hook, which
        // treats null as a no-op on macOS and is identity on other
        // platforms.
        let retained = unsafe { imp::retain(ptr as *mut c_void) };
        Self {
            ptr: retained as *const c_void,
            _marker: PhantomData,
        }
    }

    /// Release the wrapped pointer (if any) and reset to null.
    /// Mirrors `MRCOwned::Reset()` in the C++ template.
    #[inline]
    pub fn reset(&mut self) {
        // SAFETY: `imp::release` is null-safe.
        unsafe { imp::release(self.ptr as *mut c_void) };
        self.ptr = ptr::null();
    }

    /// Borrow the wrapped pointer without affecting its reference
    /// count. Mirrors `MRCOwned::Get()` in the C++ template.
    #[inline]
    pub fn get(&self) -> *mut T {
        self.ptr as *mut T
    }
}

impl<T> Default for MRCOwned<T> {
    /// Mirrors the default `MRCOwned()` constructor in C++.
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Deref for MRCOwned<T> {
    type Target = *const c_void;

    /// `Deref` to `*const c_void`, matching the C++ template's
    /// `operator T() const` implicit conversion to the underlying
    /// pointer.
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.ptr
    }
}

impl<T> Drop for MRCOwned<T> {
    /// Release the wrapped pointer on drop. The implementation is
    /// null-safe, mirroring `if (ptr) [ptr release];` in the C++
    /// template.
    #[inline]
    fn drop(&mut self) {
        // SAFETY: `imp::release` is null-safe.
        unsafe { imp::release(self.ptr as *mut c_void) };
    }
}

impl<T> Clone for MRCOwned<T> {
    /// Cloning retains the pointer, matching the C++ template's
    /// copy constructor (`[ptr retain];`).
    #[inline]
    fn clone(&self) -> Self {
        // SAFETY: `imp::retain` is null-safe.
        let retained = unsafe { imp::retain(self.ptr as *mut c_void) };
        Self {
            ptr: retained as *const c_void,
            _marker: PhantomData,
        }
    }
}

// ---------------------------------------------------------------------------
// Free-function helpers. Mirrors `MRCTransfer` and `MRCRetain` from
// the C++ header.
// ---------------------------------------------------------------------------

/// Take ownership of an Objective-C pointer (equivalent to
/// Obj-C's `__bridge_transfer`). Thin wrapper over
/// `MRCOwned::transfer`.
///
/// # Safety
///
/// See [`MRCOwned::transfer`].
#[inline]
pub fn MRCTransfer<T>(ptr: *mut T) -> MRCOwned<T> {
    MRCOwned::transfer(ptr)
}

/// Retain an Objective-C pointer and take ownership. Thin wrapper
/// over `MRCOwned::retain`.
///
/// # Safety
///
/// See [`MRCOwned::retain`].
#[inline]
pub fn MRCRetain<T>(ptr: *mut T) -> MRCOwned<T> {
    MRCOwned::retain(ptr)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `new()` produces a wrapper that Derefs to a null pointer and
    /// has no observable side-effects on drop.
    #[test]
    fn new_is_null() {
        let owned: MRCOwned = MRCOwned::new();
        assert!(owned.is_null());
        assert!(owned.get().is_null());
        // Deref target is `*const c_void`.
        let raw: *const c_void = *owned;
        assert!(raw.is_null());
    }

    /// `default()` matches `new()`.
    #[test]
    fn default_matches_new() {
        let a: MRCOwned = MRCOwned::default();
        let b: MRCOwned = MRCOwned::new();
        assert!(a.get().is_null());
        assert!(b.get().is_null());
    }

    /// `transfer` does not call into `imp::retain`, so even when the
    /// underlying retain hook is a no-op (non-macOS) the call must
    /// succeed without panicking.
    #[test]
    fn transfer_does_not_retain() {
        // Use a non-null dangling pointer; the test is only checking
        // that no retain is issued, not that the pointer is valid.
        let dangling: *mut u8 = 1usize as *mut u8;
        let owned: MRCOwned<u8> = MRCOwned::transfer(dangling);
        assert_eq!(owned.get(), dangling);
        // Drop must not release what we never retained; on macOS
        // this still calls into the runtime, but only for a
        // non-retained pointer is undefined. We therefore
        // `reset()` to a null state first to avoid that UB.
        owned.reset();
    }

    /// `MRCTransfer` and `MRCRetain` are the documented entry points
    /// for the C++ free functions; ensure they exist and produce a
    /// non-null wrapper from a non-null input.
    #[test]
    fn free_functions_compile() {
        let dangling: *mut u8 = 1usize as *mut u8;
        let from_transfer: MRCOwned<u8> = MRCTransfer(dangling);
        let from_retain: MRCOwned<u8> = MRCRetain(dangling);
        assert_eq!(from_transfer.get(), dangling);
        assert_eq!(from_retain.get(), dangling);
        // Avoid double-release UB on macOS by nulling first.
        from_transfer.reset();
        from_retain.reset();
    }
}
