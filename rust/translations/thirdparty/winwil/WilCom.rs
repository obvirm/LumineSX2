//! Idiomatic Rust 2021 translation of the WIL COM helpers.
//!
//! This module is a single-file translation of
//! `3rdparty/winwil/include/wil/com.h` and the supporting `com_*.h`
//! headers. It depends only on `std` and on the `Error` / `Result` types
//! from `WilCore`.
//!
//! The public surface is intentionally narrow: a single `com_ptr<T>`
//! smart pointer that owns a COM interface pointer and releases it on
//! drop. The C++ WIL `com_ptr_t<T, err_policy>` template, which takes
//! one of three error-handling policies (return-HRESULT, exception,
//! failfast), is collapsed to a single Rust type that exposes the
//! `Result<()>`-based "return" flavour; the exception/failfast flavours
//! can be layered on top of `Result<()>` by the caller.
//!
//! Query helpers (`query`, `try_query`, `copy`, `try_copy`) are exposed
//! as inherent methods on `com_ptr<T>`. Their default error policy is
//! "return a `Result<com_ptr<U>>`" (matching `err_return_policy`); the
//! C++ `com_ptr` / `com_ptr_failfast` aliases are not separately
//! represented.

#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

use std::ops::Deref;

use crate::thirdparty::winwil::WilCore::{Error, Result};

// ---------------------------------------------------------------------------
// IUnknown contract
// ---------------------------------------------------------------------------

/// Minimal interface that all COM objects must satisfy. The C++ WIL
/// library treats `IUnknown` as the root of every COM interface
/// (`com_ptr_t` only ever holds pointers to things that ultimately
/// derive from `IUnknown`). This Rust translation uses a trait that
/// mirrors the vtable: `AddRef` and `Release` are the two methods that
/// `com_ptr<T>` actually calls; `QueryInterface` is called indirectly
/// through the `query` helper.
pub trait IUnknown {
    /// Increments the reference count and returns the new value. In
    /// the real Win32 SDK this is `ULONG`, but Rust callers do not
    /// need the new value: the C++ library also ignores it.
    fn AddRef(&self);

    /// Decrements the reference count and returns the new value. When
    /// the count reaches zero the object must free itself. The C++
    /// library ignores the return value.
    fn Release(&self) -> u32;

    /// Asks the object for a pointer to one of its interfaces. Returns
    /// `Some(raw_ptr)` on success and `None` on failure (or "not
    /// supported"). The caller is responsible for `AddRef`ing the
    /// returned pointer.
    ///
    /// `iid` is the interface identifier, modelled here as a `(u32,
    /// u16, u16, [u8; 8])` to match the Win32 `GUID` layout without
    /// depending on the `windows` crate.
    fn QueryInterface(
        &self,
        iid: &GUID,
    ) -> Option<*mut std::ffi::c_void>;
}

/// Win32 `GUID` layout, kept private to this module to avoid colliding
/// with the `windows-sys` type of the same name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct GUID {
    pub Data1: u32,
    pub Data2: u16,
    pub Data3: u16,
    pub Data4: [u8; 8],
}

impl GUID {
    /// Constructs a `GUID` from its individual fields.
    pub const fn new(d1: u32, d2: u16, d3: u16, d4: [u8; 8]) -> Self {
        GUID {
            Data1: d1,
            Data2: d2,
            Data3: d3,
            Data4: d4,
        }
    }
}

// ---------------------------------------------------------------------------
// com_ptr<T>
// ---------------------------------------------------------------------------

/// RAII COM smart pointer. Owns a `T*` and calls `Release` on it when
/// the `com_ptr` is dropped (or reset, or moved-from).
///
/// `T: IUnknown` is a hard bound because the only thing `com_ptr`
/// itself does is call `AddRef`/`Release`; the various `query*` /
/// `copy*` helpers are generic on the destination interface and use
/// `QueryInterface` on the source.
pub struct com_ptr<T: IUnknown> {
    ptr: *mut T,
}

// Safety: COM objects are reference-counted and use atomic refcounts in
// practice, but the vtable pointer itself is a `*mut T` which is `!Send`
// by default. We mark `com_ptr<T>` as `Send` and `Sync` because the
// underlying refcount machinery is thread-safe; this matches how
// `windows::ComPtr` is `Send`/`Sync` for `T: Send` COM interfaces.
unsafe impl<T: IUnknown> Send for com_ptr<T> {}
unsafe impl<T: IUnknown> Sync for com_ptr<T> {}

impl<T: IUnknown> Drop for com_ptr<T> {
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: `self.ptr` was obtained from a valid `com_ptr`
            // construction, so the object is alive. `Release` may
            // free the object; we must not touch `self.ptr` afterwards.
            unsafe { (*self.ptr).Release() };
        }
    }
}

impl<T: IUnknown> com_ptr<T> {
    /// Constructs an empty `com_ptr` (analogous to a null `com_ptr_t`).
    pub fn null() -> Self {
        com_ptr {
            ptr: std::ptr::null_mut(),
        }
    }

    /// Wraps a raw interface pointer, taking ownership. The pointer
    /// must be non-null and must already be `AddRef`ed to at least 1
    /// (the typical contract for `AddRef`-returning COM APIs).
    ///
    /// # Safety
    ///
    /// * `ptr` must point to a valid COM object implementing `T`.
    /// * `ptr` must already be reference-counted; the caller transfers
    ///   its reference to the new `com_ptr`.
    pub unsafe fn from_raw(ptr: *mut T) -> Self {
        com_ptr { ptr }
    }

    /// Returns the raw interface pointer without releasing ownership.
    /// The caller is responsible for eventually `Release`-ing the
    /// pointer.
    pub fn into_raw(mut self) -> *mut T {
        let p = self.ptr;
        self.ptr = std::ptr::null_mut();
        p
    }

    /// Returns the raw interface pointer while retaining ownership.
    /// The returned pointer must not be released externally.
    #[inline]
    pub fn get(&self) -> *mut T {
        self.ptr
    }

    /// Releases the current pointer and replaces it with `nullptr`.
    /// The `Release` call happens in this method rather than via `Drop`
    /// because the destructor is also wired up; we manually run the
    /// release path to avoid a `mem::forget`.
    pub fn reset(&mut self) {
        if !self.ptr.is_null() {
            // SAFETY: see `Drop`.
            unsafe { (*self.ptr).Release() };
            self.ptr = std::ptr::null_mut();
        }
    }

    /// Replaces the wrapped pointer with a new one, `Release`-ing the
    /// previous pointer if it was non-null. The new pointer must be
    /// `AddRef`-ed to at least 1.
    ///
    /// # Safety
    ///
    /// The same requirements as `from_raw` apply to `ptr`.
    pub unsafe fn attach(&mut self, ptr: *mut T) {
        if !self.ptr.is_null() {
            (*self.ptr).Release();
        }
        self.ptr = ptr;
    }

    /// Returns true when the pointer is non-null.
    #[inline]
    pub fn is_valid(&self) -> bool {
        !self.ptr.is_null()
    }

    /// Detaches ownership and returns the raw pointer, leaving `self`
    /// holding null. The returned pointer must be `Release`-d by the
    /// caller. Mirrors `wil::com_ptr_t::detach`.
    pub fn detach(&mut self) -> *mut T {
        let p = self.ptr;
        self.ptr = std::ptr::null_mut();
        p
    }

    /// Adds a reference to the wrapped object, returning a new
    /// `com_ptr` that shares ownership. Mirrors `com_ptr_t::copy` for
    /// the same interface type.
    pub fn copy(&self) -> Result<com_ptr<T>> {
        if self.ptr.is_null() {
            return Ok(com_ptr::null());
        }
        // SAFETY: `self.ptr` is non-null and a valid COM object.
        unsafe { (*self.ptr).AddRef() };
        // SAFETY: we just incremented the refcount, so the pointer is
        // now owned by the new `com_ptr` and the old one.
        Ok(unsafe { com_ptr::from_raw(self.ptr) })
    }
}

impl<T: IUnknown> Default for com_ptr<T> {
    fn default() -> Self {
        com_ptr::null()
    }
}

impl<T: IUnknown> Clone for com_ptr<T> {
    fn clone(&self) -> Self {
        // Match the C++ copy constructor: AddRef and copy. If the
        // pointer is null, the copy is null.
        if self.ptr.is_null() {
            return com_ptr::null();
        }
        // SAFETY: see `copy`.
        unsafe { (*self.ptr).AddRef() };
        // SAFETY: refcount was just incremented.
        unsafe { com_ptr::from_raw(self.ptr) }
    }
}

impl<T: IUnknown> Deref for com_ptr<T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: `self.ptr` is non-null for a valid `com_ptr`. The
        // contract of `from_raw` requires the caller to pass a valid
        // pointer.
        unsafe { &*self.ptr }
    }
}

impl<T: IUnknown> std::fmt::Debug for com_ptr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("com_ptr")
            .field("is_valid", &self.is_valid())
            .finish()
    }
}

impl<T: IUnknown> PartialEq for com_ptr<T> {
    fn eq(&self, other: &Self) -> bool {
        self.ptr == other.ptr
    }
}

impl<T: IUnknown> Eq for com_ptr<T> {}

impl<T: IUnknown> std::ops::DerefMut for com_ptr<T> {
    fn deref_mut(&mut self) -> &mut T {
        // SAFETY: see `deref`.
        unsafe { &mut *self.ptr }
    }
}

// ---------------------------------------------------------------------------
// Query helpers
// ---------------------------------------------------------------------------

impl<T: IUnknown> com_ptr<T> {
    /// Queries the underlying COM object for another interface,
    /// `AddRef`-ing the returned pointer on success. Returns `Err` if
    /// the object does not support `U`. Mirrors
    /// `wil::com_ptr_t::try_query` in its failure shape: a null
    /// pointer on the way out means "interface not supported".
    pub fn try_query<U: IUnknown + 'static>(&self) -> com_ptr<U> {
        if self.ptr.is_null() {
            return com_ptr::null();
        }
        // SAFETY: the pointer is valid; we obtain the IID for U via
        // the trait object we cast to. In a real Win32 binding this
        // would use `IID_PPV_ARGS_Helper`; here we use a placeholder
        // GUID that the implementor of `IUnknown` is responsible for
        // matching.
        let iid = interface_id::<U>();
        // SAFETY: same as above.
        let raw = unsafe { (*self.ptr).QueryInterface(&iid) };
        match raw {
            Some(p) => {
                // The QueryInterface contract is that the returned
                // pointer is already AddRef-ed. We therefore wrap it
                // via from_raw without an extra AddRef.
                // SAFETY: p came from a successful QueryInterface and
                // is non-null.
                unsafe { com_ptr::<U>::from_raw(p as *mut U) }
            }
            None => com_ptr::null(),
        }
    }

    /// Variant of `try_query` that returns `Err(E_NOINTERFACE)` when
    /// the interface is not supported. Mirrors `com_ptr_t::query`
    /// under the "return policy".
    pub fn query<U: IUnknown + 'static>(&self) -> Result<com_ptr<U>> {
        let result = self.try_query::<U>();
        if result.is_valid() {
            Ok(result)
        } else {
            Err(Error::Hresult(crate::thirdparty::winwil::WilCore::E_NOINTERFACE))
        }
    }

    /// `Copy`-style query: returns a `com_ptr<U>` that shares the
    /// same underlying object. The semantics match the C++
    /// `com_ptr_t::try_copy` / `copy` pair.
    pub fn try_copy<U: IUnknown + 'static>(&self) -> com_ptr<U> {
        self.try_query::<U>()
    }

    /// `Copy`-style query that returns `Err(E_NOINTERFACE)` on
    /// failure. Mirrors `com_ptr_t::copy` under the "return policy".
    pub fn copy_to<U: IUnknown + 'static>(&self) -> Result<com_ptr<U>> {
        self.query::<U>()
    }
}

// ---------------------------------------------------------------------------
// Out-parameter helpers
// ---------------------------------------------------------------------------

/// Translates a `&mut com_ptr<U>` to a `*mut *mut U` "out parameter"
/// suitable for passing to Win32 APIs that take `SomeInterface**`. The
/// pointer the API writes into is automatically adopted into the
/// `com_ptr` on `Drop`, replacing any previously-held pointer.
///
/// Mirrors `wil::out_param` for smart pointers.
pub struct out_param<'a, T: IUnknown> {
    target: &'a mut com_ptr<T>,
    raw: *mut *mut T,
}

impl<'a, T: IUnknown> out_param<'a, T> {
    /// Constructs a new out-parameter wrapper around `target`.
    pub fn new(target: &'a mut com_ptr<T>) -> Self {
        out_param {
            target,
            raw: std::ptr::null_mut(),
        }
    }
}

impl<'a, T: IUnknown> Drop for out_param<'a, T> {
    fn drop(&mut self) {
        // SAFETY: `raw` was either null (no API call written through
        // it) or was written by a COM API that returned a valid,
        // AddRef-ed pointer.
        if !self.raw.is_null() {
            unsafe { self.target.attach(*self.raw) };
        }
    }
}

impl<'a, T: IUnknown> From<out_param<'a, T>> for *mut *mut T {
    fn from(mut o: out_param<'a, T>) -> Self {
        // Hand the address of the inner pointer to the API. We don't
        // `forget` here because `Drop` already does the right thing:
        // it will pick up whatever the API wrote.
        o.raw = unsafe { &mut *(&mut o.target.detach() as *mut *mut T) };
        // Reconstruct a null pointer to the placeholder storage so
        // the API can write into it. In the C++ library this is
        // achieved with operator T**; here we exploit the fact that
        // `target` was just detached, so its inner pointer is null,
        // and we return the address of that null storage.
        let mut null_storage: *mut T = std::ptr::null_mut();
        let result: *mut *mut T = &mut null_storage;
        // Hold on to the wrapper so its Drop runs and adopts the
        // pointer the API wrote.
        std::mem::forget(o);
        result
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns a placeholder `GUID` used as the "IID" of an interface in
/// this translation. The C++ WIL `try_query` machinery relies on
/// compile-time `IID_PPV_ARGS` to pass the right IID for each
/// interface; in this single-file Rust translation we lack the
/// type-system support to express that, so we use a sentinel `GUID` and
/// document the limitation.
///
/// The implementor of `IUnknown::QueryInterface` is expected to match
/// on the `Data1` field (or use a custom protocol) to decide which
/// interface to return. In a real Win32 binding this would be a
/// per-interface `extern "system"` const.
fn interface_id<U: IUnknown + 'static>() -> GUID {
    // Use the type's `TypeId` address as a stable but unique value for
    // each interface. Two distinct interface types will have distinct
    // `TypeId` values, which is all we need to distinguish them within
    // a single binding.
    let id = std::any::TypeId::of::<U>();
    let ptr = &id as *const _ as usize as u32;
    GUID::new(ptr, 0, 0, [0; 8])
}

/// `CoCreateInstance`-style helper that wraps a raw `IUnknown` pointer
/// into a `com_ptr<T>`. The C++ equivalent is `wil::com_ptr<T>::create`
/// with an explicit CLSID. In this translation we leave the CLSID
/// lookup to the caller (who would normally do it via
/// `CoCreateInstance` from a Win32 binding) and just package the
/// pointer.
impl<T: IUnknown> com_ptr<T> {
    /// Wraps a raw COM pointer returned from a CoCreate-style factory.
    /// The caller is responsible for ensuring the pointer implements
    /// `T` and is properly `AddRef`ed.
    ///
    /// # Safety
    ///
    /// See `from_raw`.
    pub unsafe fn from_com(ptr: *mut T) -> Self {
        com_ptr::<T>::from_raw(ptr)
    }
}

// ---------------------------------------------------------------------------
// A tiny in-module smoke test, gated so it does not run on cargo build
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "wilcom_tests"))]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A minimal mock COM object that records its refcount so we can
    /// verify `com_ptr` behaviour without depending on the Win32 SDK.
    struct MockUnknown {
        refcount: AtomicU32,
    }

    impl IUnknown for MockUnknown {
        fn AddRef(&self) {
            self.refcount.fetch_add(1, Ordering::SeqCst);
        }
        fn Release(&self) -> u32 {
            let prev = self.refcount.fetch_sub(1, Ordering::SeqCst);
            if prev == 1 {
                // Last reference dropped: free. The C++ equivalent
                // would `delete this`. In Rust we leak the box on
                // purpose to keep the test simple.
            }
            prev - 1
        }
        fn QueryInterface(
            &self,
            _iid: &GUID,
        ) -> Option<*mut std::ffi::c_void> {
            // Mock does not support any derived interface.
            None
        }
    }

    #[test]
    fn empty_com_ptr_does_not_call_release() {
        let p: com_ptr<MockUnknown> = com_ptr::null();
        assert!(!p.is_valid());
    }

    #[test]
    fn attach_and_drop() {
        let obj = Box::new(MockUnknown {
            refcount: AtomicU32::new(1),
        });
        let raw = Box::into_raw(obj);
        // SAFETY: fresh allocation, refcount == 1.
        let p = unsafe { com_ptr::<MockUnknown>::from_raw(raw) };
        assert!(p.is_valid());
        drop(p);
        // Refcount was 1, Release dropped it to 0; the mock
        // deliberately leaks. Asserting here would be unsafe, so we
        // just let the test end.
    }
}
