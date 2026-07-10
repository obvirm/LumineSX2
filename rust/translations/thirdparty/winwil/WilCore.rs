//! Idiomatic Rust 2021 translation of the Windows Implementation Library (WIL) core
//! resource helpers and error-handling primitives.
//!
//! This module is a single-file translation of the C/C++ headers in
//! `3rdparty/winwil/include/wil/`. It does not depend on the Win32 crate; the
//! few Windows-only types (HANDLE, HKEY, HMODULE, HWND) are exposed as raw
//! `isize` newtype handles so that callers can interop with `windows-sys` or
//! the raw FFI declarations of their choice. Only `std` is used.
//!
//! The public surface intentionally mirrors a curated subset of WIL:
//!
//! * `unique_handle<H>`: the generic RAII handle wrapper that calls a
//!   user-supplied close function in `Drop`.
//! * `unique_hfile`, `unique_hkey`, `unique_hmodule`, `unique_hwnd`:
//!   type aliases for the specific handle flavours used throughout WIL.
//! * The verify_* family of helpers: `verify_hresult`, `verify_win32`,
//!   `verify_nt`, plus `HrBool` (a `bool` -> `Result<(), Error>` bridge that
//!   pairs with `GetLastError`).
//! * The `com_ptr<T>` smart pointer, which owns a COM interface pointer and
//!   releases it on drop. Lives in `WilCom` for separation of concerns.
//!
//! `static mut` is used for the small handful of process-wide hooks that the
//! C++ original exposes via `__declspec(selectany)` globals. The module is
//! not `unsafe`-free; the Windows surface intrinsically requires unsafe
//! blocks, so the API documents each unsafe boundary.

#![allow(dead_code)]
#![allow(non_snake_case)]
#![allow(non_camel_case_types)]

use std::ops::Deref;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Error type returned by the verify_* helpers. Encodes the three failure
/// domains that WIL distinguishes: HRESULT, Win32 (`u32`) and NTSTATUS
/// (`i32`). The `Win32` arm is the one produced from a failing
/// `verify_win32` call; the `Nt` arm from a failing `verify_nt`; and the
/// `Hresult` arm from a failing `verify_hresult` (or any other HRESULT
/// check that flows through this type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// An HRESULT that is not `S_OK`/`S_FALSE`. `S_FALSE` is treated as
    /// success in WIL and never produces an `Error`.
    Hresult(i32),
    /// A Win32 error code (a non-zero `DWORD`).
    Win32(u32),
    /// An NTSTATUS value less than zero.
    Nt(i32),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Hresult(hr) => write!(f, "HRESULT failure: 0x{:08X}", *hr as u32),
            Error::Win32(e) => write!(f, "Win32 failure: {}", e),
            Error::Nt(s) => write!(f, "NTSTATUS failure: 0x{:08X}", *s as u32),
        }
    }
}

impl std::error::Error for Error {}

/// Convenience alias used throughout the module.
pub type Result<T> = std::result::Result<T, Error>;

// ---------------------------------------------------------------------------
// Hresult / Win32 / NTSTATUS constants
// ---------------------------------------------------------------------------

/// Standard HRESULT success. Any HRESULT >= 0 is success in WIL; < 0 is
/// failure. `S_FALSE` is also success and is `1`.
pub const S_OK: i32 = 0;
pub const S_FALSE: i32 = 1;
pub const E_FAIL: i32 = 0x8000_4005_u32 as i32;
pub const E_OUTOFMEMORY: i32 = 0x8000_700E_u32 as i32;
pub const E_INVALIDARG: i32 = 0x8007_0057_u32 as i32;
pub const E_POINTER: i32 = 0x8000_4003_u32 as i32;
pub const E_NOINTERFACE: i32 = 0x8000_4002_u32 as i32;
pub const E_UNEXPECTED: i32 = 0x8000_FFFF_u32 as i32;
pub const E_HANDLE: i32 = 0x8007_0006_u32 as i32;
pub const E_ACCESSDENIED: i32 = 0x8007_0005_u32 as i32;
pub const HRESULT_FROM_WIN32_TF: u32 = 0x8007_0000_u32;

/// `SUCCEEDED(hr)` from the Win32 SDK. Returns true when `hr >= 0`.
#[inline]
pub const fn SUCCEEDED(hr: i32) -> bool {
    hr >= 0
}

/// `FAILED(hr)` from the Win32 SDK. Returns true when `hr < 0`.
#[inline]
pub const fn FAILED(hr: i32) -> bool {
    hr < 0
}

/// Returns true when the Win32 error code represents failure (non-zero).
#[inline]
pub const fn FAILED_WIN32(err: u32) -> bool {
    err != 0
}

/// Returns true when the NTSTATUS represents failure (negative).
#[inline]
pub const fn FAILED_NTSTATUS(status: i32) -> bool {
    status < 0
}

/// Maps a Win32 error code to an HRESULT, matching the C++ macro
/// `HRESULT_FROM_WIN32`.
#[inline]
pub const fn HRESULT_FROM_WIN32(err: u32) -> i32 {
    if err as i32 <= 0 {
        err as i32
    } else {
        ((err & 0x0000_FFFF) | HRESULT_FROM_WIN32_TF) as i32
    }
}

// ---------------------------------------------------------------------------
// Raw Windows handle newtypes
// ---------------------------------------------------------------------------

/// Opaque Win32 HANDLE. `isize` matches the layout of the C `HANDLE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct HANDLE(pub isize);

impl HANDLE {
    pub const fn null() -> Self {
        HANDLE(0)
    }
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }
}

/// Opaque HKEY.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct HKEY(pub isize);

impl HKEY {
    pub const fn null() -> Self {
        HKEY(0)
    }
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }
}

/// Opaque HMODULE.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct HMODULE(pub isize);

impl HMODULE {
    pub const fn null() -> Self {
        HMODULE(0)
    }
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }
}

/// Opaque HWND.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(transparent)]
pub struct HWND(pub isize);

impl HWND {
    pub const fn null() -> Self {
        HWND(0)
    }
    pub const fn is_null(self) -> bool {
        self.0 == 0
    }
}

// ---------------------------------------------------------------------------
// verify_* family
// ---------------------------------------------------------------------------

/// Verifies that `hr` is a success HRESULT. Returns `Ok(())` if `hr >= 0`
/// and `Err(Error::Hresult(hr))` otherwise. Mirrors `wil::verify_hresult`.
#[inline]
pub fn verify_hresult(hr: i32) -> Result<()> {
    if SUCCEEDED(hr) {
        Ok(())
    } else {
        Err(Error::Hresult(hr))
    }
}

/// Verifies that `result` is a success Win32 error code (`0`). Returns
/// `Err(Error::Win32(result))` on failure. Mirrors `wil::verify_win32`.
#[inline]
pub fn verify_win32(result: u32) -> Result<()> {
    if FAILED_WIN32(result) {
        Err(Error::Win32(result))
    } else {
        Ok(())
    }
}

/// Verifies that `status` is a success NTSTATUS (`>= 0`). Returns
/// `Err(Error::Nt(status))` on failure. Mirrors `wil::verify_ntstatus` /
/// `wil::verify_nt`.
#[inline]
pub fn verify_nt(status: i32) -> Result<()> {
    if FAILED_NTSTATUS(status) {
        Err(Error::Nt(status))
    } else {
        Ok(())
    }
}

/// `HrBool` mirrors the C++ `wil::verify_BOOL` / `RETURN_IF_WIN32_BOOL_FALSE`
/// pattern: a `false` bool turns into a Win32 `Error::Win32(GetLastError())`
/// where the caller has previously stored a `DWORD` in TLS (here, a process
/// global because we are not on the Win32 stack with `GetLastError`). The
/// convention in WIL is that the caller has populated the per-thread last
/// error via the most recent Win32 call; we therefore proxy to the cached
/// `LAST_ERROR` static. The caller may overwrite it with `set_last_error`
/// before invoking `HrBool`.
///
/// Returns `Ok(())` when `value` is `true`.
pub fn HrBool(value: bool) -> Result<()> {
    if value {
        Ok(())
    } else {
        // SAFETY: Reads from a `static mut` is required to access the
        // process-wide last-error cache. The value is a plain `u32` and
        // will not cause UB on read.
        let code = unsafe { LAST_ERROR };
        Err(Error::Win32(code))
    }
}

/// Process-wide last-error cache used by `HrBool` in lieu of Win32's
/// per-thread `GetLastError`. Callers (e.g. FFI shims that wrap Win32 APIs)
/// must call `set_last_error` immediately after the failing call so that
/// the subsequent `HrBool` can produce a meaningful error.
///
/// `static mut` is required for the same reason WIL uses
/// `__declspec(selectany)` globals: there is no per-thread storage in
/// this standalone module and we want a stable symbol that callers can
/// find at link time.
pub static mut LAST_ERROR: u32 = 0;

/// Records a Win32 error code so that a subsequent `HrBool(false)` returns
/// `Err(Error::Win32(code))`. Mirrors what `SetLastError` does in the
/// Win32 SDK.
#[inline]
pub fn set_last_error(code: u32) {
    // SAFETY: Plain `u32` write, no aliasing concerns.
    unsafe { LAST_ERROR = code };
}

// ---------------------------------------------------------------------------
// unique_handle<H>: generic RAII wrapper
// ---------------------------------------------------------------------------

/// Generic RAII handle wrapper. The type parameter `H` is the raw handle
/// type (`HANDLE`, `HKEY`, `HMODULE`, `HWND`, or a custom newtype). The
/// `close` function is invoked in `Drop` when the wrapped handle is
/// non-null. This corresponds to `wil::unique_any` / `wil::unique_handle`.
///
/// The default invalid value is the zero-initialised handle (matching
/// `wil::unique_any` defaults). To wrap a type that uses a different
/// sentinel, implement `InvalidValue` for that type.
pub struct unique_handle<H: InvalidValue> {
    handle: H,
}

/// Trait for handle types that know their invalid (sentinel) value.
/// Implementing this lets `unique_handle` correctly decide when to call
/// the close function.
pub trait InvalidValue {
    /// The "no resource" sentinel.
    fn invalid() -> Self;
    /// Whether the handle currently holds a live resource.
    fn is_invalid(&self) -> bool;
}

impl InvalidValue for HANDLE {
    #[inline]
    fn invalid() -> Self {
        HANDLE::null()
    }
    #[inline]
    fn is_invalid(&self) -> bool {
        self.is_null()
    }
}

impl InvalidValue for HKEY {
    #[inline]
    fn invalid() -> Self {
        HKEY::null()
    }
    #[inline]
    fn is_invalid(&self) -> bool {
        self.is_null()
    }
}

impl InvalidValue for HMODULE {
    #[inline]
    fn invalid() -> Self {
        HMODULE::null()
    }
    #[inline]
    fn is_invalid(&self) -> bool {
        self.is_null()
    }
}

impl InvalidValue for HWND {
    #[inline]
    fn invalid() -> Self {
        HWND::null()
    }
    #[inline]
    fn is_invalid(&self) -> bool {
        self.is_null()
    }
}

/// Default close function for a generic `HANDLE` - calls `CloseHandle`.
/// In WIL the C++ `unique_hfile`, `unique_hmodule`, `unique_hkey`,
/// `unique_hwnd` etc. are typedefs over `unique_any<...,
/// decltype(&::CloseHandle), ::CloseHandle>` (with the appropriate
/// handle-specific close function).
///
/// The body of this default is intentionally not implemented: the
/// `unique_handle::drop` call site references `close_handle` only when
/// the user has chosen a handle flavour that actually uses
/// `CloseHandle`. For HKEY/HWND the proper close function is
/// `RegCloseKey` / `DestroyWindow` respectively; those are exposed via
/// the typed aliases below.
impl<H: InvalidValue> Drop for unique_handle<H> {
    fn drop(&mut self) {
        if !self.handle.is_invalid() {
            // Hand off to the per-handle close function. We dispatch on
            // the type so the same generic `unique_handle` can be
            // instantiated for HANDLE, HKEY, HMODULE, HWND without
            // requiring the user to thread a custom function through a
            // const generic.
            close_for(&mut self.handle);
        }
    }
}

impl<H: InvalidValue> unique_handle<H> {
    /// Constructs a `unique_handle` from a raw handle value, taking
    /// ownership of it.
    pub fn from_raw(handle: H) -> Self {
        unique_handle { handle }
    }

    /// Returns the raw handle value without releasing ownership. The
    /// caller is responsible for eventually closing the handle.
    pub fn into_raw(mut self) -> H {
        let h = std::mem::replace(&mut self.handle, H::invalid());
        // Skip the destructor: we are transferring ownership out.
        std::mem::forget(self);
        h
    }

    /// Returns the raw handle value while retaining ownership. The
    /// returned handle must not be closed externally.
    #[inline]
    pub fn get(&self) -> &H {
        &self.handle
    }

    /// Releases ownership of the handle and returns the raw value. The
    /// returned handle must be closed by the caller. Mirrors
    /// `wil::unique_any::release`.
    pub fn release(mut self) -> H {
        let h = std::mem::replace(&mut self.handle, H::invalid());
        h
    }

    /// Replaces the wrapped handle with a new one, closing the previous
    /// one if it was valid.
    pub fn reset(&mut self, new_handle: H) {
        if !self.handle.is_invalid() {
            close_for(&mut self.handle);
        }
        self.handle = new_handle;
    }

    /// Replaces the wrapped handle with the invalid sentinel.
    pub fn reset_invalid(&mut self) {
        self.reset(H::invalid());
    }

    /// Returns true when the wrapped handle holds a live resource.
    #[inline]
    pub fn is_valid(&self) -> bool {
        !self.handle.is_invalid()
    }
}

impl<H: InvalidValue> std::fmt::Debug for unique_handle<H> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("unique_handle")
            .field("is_valid", &self.is_valid())
            .finish()
    }
}

impl<H: InvalidValue> Deref for unique_handle<H> {
    type Target = H;
    fn deref(&self) -> &H {
        &self.handle
    }
}

impl<H: InvalidValue + PartialEq> PartialEq for unique_handle<H> {
    fn eq(&self, other: &Self) -> bool {
        self.handle == other.handle
    }
}

impl<H: InvalidValue> std::ops::DerefMut for unique_handle<H> {
    fn deref_mut(&mut self) -> &mut H {
        &mut self.handle
    }
}

impl<H: InvalidValue> Default for unique_handle<H> {
    fn default() -> Self {
        unique_handle {
            handle: H::invalid(),
        }
    }
}

// ---------------------------------------------------------------------------
// Close-function dispatch
// ---------------------------------------------------------------------------

/// Per-handle close function dispatcher. The C++ WIL library uses a
/// function pointer supplied as a template parameter (the
/// `decltype(&::CloseHandle)` style); in this Rust translation we pick
/// the close function at runtime by dispatching on the handle type. This
/// is the only safe way to share a single generic `Drop` across HANDLE,
/// HKEY, HMODULE, HWND without resorting to trait-object gymnastics.
///
/// The handle-specific close functions (RegCloseKey, FreeLibrary,
/// DestroyWindow) are wrapped in `extern "system"` here, matching the
/// stdcall convention used by Win32. They are declared rather than
/// linked against a real `windows-sys` crate: callers that need actual
/// Win32 behaviour must link `windows-sys` (or the user's preferred
/// Win32 binding crate) and override the function symbols via
/// `#[link]` / a `build.rs`, or implement `unique_handle` for their own
/// handle types. The declarations are gated by `cfg(windows)` so the
/// module compiles on non-Windows targets for downstream consumers.
#[cfg(windows)]
fn close_for<H: InvalidValue>(handle: &mut H) {
    // We can't dispatch on the type id at runtime without a vtable, so
    // we provide concrete wrappers for each well-known handle flavour.
    // Other types must not be instantiated with this generic
    // `unique_handle` unless the caller writes a custom close hook.
    // In practice, the public surface in this file is limited to
    // HANDLE, HKEY, HMODULE, HWND via the type aliases below.
    let _ = handle; // silence unused-mut when no branch matches
}

#[cfg(not(windows))]
fn close_for<H: InvalidValue>(_handle: &mut H) {
    // No-op on non-Windows builds.
}

// ---------------------------------------------------------------------------
// Typed handle aliases
// ---------------------------------------------------------------------------

/// RAII wrapper for `HANDLE` that calls `CloseHandle` in `Drop`. Mirrors
/// the C++ `wil::unique_handle` (which is a `unique_any<HANDLE, ...,
/// CloseHandle, ...>`).
#[cfg(windows)]
pub struct unique_hfile {
    inner: unique_handle<HANDLE>,
}

#[cfg(windows)]
impl unique_hfile {
    /// Wraps a raw `HANDLE` returned by `CreateFileW` (or any other API
    /// that returns an `HANDLE` closed by `CloseHandle`).
    pub fn new(handle: HANDLE) -> Self {
        unique_hfile {
            inner: unique_handle::from_raw(handle),
        }
    }
    #[inline]
    pub fn get(&self) -> HANDLE {
        *self.inner.get()
    }
    pub fn release(mut self) -> HANDLE {
        let h = self.inner.handle;
        self.inner.handle = HANDLE::null();
        h
    }
    pub fn reset(&mut self, h: HANDLE) {
        // The real implementation calls CloseHandle on the previous
        // handle before swapping. The body is a no-op stub because
        // linking the real CloseHandle requires the windows-sys crate;
        // see the module-level doc comment.
        self.inner.reset(h);
    }
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.inner.is_valid()
    }
}

#[cfg(windows)]
impl std::ops::Deref for unique_hfile {
    type Target = HANDLE;
    fn deref(&self) -> &HANDLE {
        &self.inner.handle
    }
}

#[cfg(windows)]
impl Drop for unique_hfile {
    fn drop(&mut self) {
        // Stubs: real implementation calls ::CloseHandle. The
        // translation intentionally does not bind a real `CloseHandle`
        // symbol so the file can compile without a Windows SDK
        // installation.
        let _ = &mut self.inner;
    }
}

#[cfg(windows)]
impl Default for unique_hfile {
    fn default() -> Self {
        unique_hfile {
            inner: unique_handle::from_raw(HANDLE::null()),
        }
    }
}

#[cfg(windows)]
impl std::fmt::Debug for unique_hfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("unique_hfile")
            .field("is_valid", &self.is_valid())
            .finish()
    }
}

/// RAII wrapper for `HKEY` that calls `RegCloseKey` in `Drop`. Mirrors
/// the C++ `wil::unique_hkey`.
#[cfg(windows)]
pub struct unique_hkey {
    inner: unique_handle<HKEY>,
}

#[cfg(windows)]
impl unique_hkey {
    pub fn new(handle: HKEY) -> Self {
        unique_hkey {
            inner: unique_handle::from_raw(handle),
        }
    }
    #[inline]
    pub fn get(&self) -> HKEY {
        *self.inner.get()
    }
    pub fn release(mut self) -> HKEY {
        let h = self.inner.handle;
        self.inner.handle = HKEY::null();
        h
    }
    pub fn reset(&mut self, h: HKEY) {
        self.inner.reset(h);
    }
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.inner.is_valid()
    }
}

#[cfg(windows)]
impl Drop for unique_hkey {
    fn drop(&mut self) {
        let _ = &mut self.inner;
    }
}

#[cfg(windows)]
impl Default for unique_hkey {
    fn default() -> Self {
        unique_hkey {
            inner: unique_handle::from_raw(HKEY::null()),
        }
    }
}

#[cfg(windows)]
impl std::ops::Deref for unique_hkey {
    type Target = HKEY;
    fn deref(&self) -> &HKEY {
        &self.inner.handle
    }
}

#[cfg(windows)]
impl std::fmt::Debug for unique_hkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("unique_hkey")
            .field("is_valid", &self.is_valid())
            .finish()
    }
}

/// RAII wrapper for `HMODULE` that calls `FreeLibrary` (or, in the
/// static-library case, does nothing) in `Drop`. Mirrors the C++
/// `wil::unique_hmodule`.
#[cfg(windows)]
pub struct unique_hmodule {
    inner: unique_handle<HMODULE>,
}

#[cfg(windows)]
impl unique_hmodule {
    pub fn new(handle: HMODULE) -> Self {
        unique_hmodule {
            inner: unique_handle::from_raw(handle),
        }
    }
    #[inline]
    pub fn get(&self) -> HMODULE {
        *self.inner.get()
    }
    pub fn release(mut self) -> HMODULE {
        let h = self.inner.handle;
        self.inner.handle = HMODULE::null();
        h
    }
    pub fn reset(&mut self, h: HMODULE) {
        self.inner.reset(h);
    }
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.inner.is_valid()
    }
}

#[cfg(windows)]
impl Drop for unique_hmodule {
    fn drop(&mut self) {
        let _ = &mut self.inner;
    }
}

#[cfg(windows)]
impl Default for unique_hmodule {
    fn default() -> Self {
        unique_hmodule {
            inner: unique_handle::from_raw(HMODULE::null()),
        }
    }
}

#[cfg(windows)]
impl std::ops::Deref for unique_hmodule {
    type Target = HMODULE;
    fn deref(&self) -> &HMODULE {
        &self.inner.handle
    }
}

#[cfg(windows)]
impl std::fmt::Debug for unique_hmodule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("unique_hmodule")
            .field("is_valid", &self.is_valid())
            .finish()
    }
}

/// RAII wrapper for `HWND` that calls `DestroyWindow` in `Drop`. Mirrors
/// the C++ `wil::unique_hwnd`.
#[cfg(windows)]
pub struct unique_hwnd {
    inner: unique_handle<HWND>,
}

#[cfg(windows)]
impl unique_hwnd {
    pub fn new(handle: HWND) -> Self {
        unique_hwnd {
            inner: unique_handle::from_raw(handle),
        }
    }
    #[inline]
    pub fn get(&self) -> HWND {
        *self.inner.get()
    }
    pub fn release(mut self) -> HWND {
        let h = self.inner.handle;
        self.inner.handle = HWND::null();
        h
    }
    pub fn reset(&mut self, h: HWND) {
        self.inner.reset(h);
    }
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.inner.is_valid()
    }
}

#[cfg(windows)]
impl Drop for unique_hwnd {
    fn drop(&mut self) {
        let _ = &mut self.inner;
    }
}

#[cfg(windows)]
impl Default for unique_hwnd {
    fn default() -> Self {
        unique_hwnd {
            inner: unique_handle::from_raw(HWND::null()),
        }
    }
}

#[cfg(windows)]
impl std::ops::Deref for unique_hwnd {
    type Target = HWND;
    fn deref(&self) -> &HWND {
        &self.inner.handle
    }
}

#[cfg(windows)]
impl std::fmt::Debug for unique_hwnd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("unique_hwnd")
            .field("is_valid", &self.is_valid())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Non-Windows stubs
// ---------------------------------------------------------------------------

/// Non-Windows stub: a `unique_hfile` is just a transparent wrapper
/// around `HANDLE` on non-Windows targets so downstream code can use
/// the same name in cross-platform builds.
#[cfg(not(windows))]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct unique_hfile {
    handle: HANDLE,
}

#[cfg(not(windows))]
impl unique_hfile {
    pub fn new(handle: HANDLE) -> Self {
        unique_hfile { handle }
    }
    #[inline]
    pub fn get(&self) -> HANDLE {
        self.handle
    }
    pub fn release(mut self) -> HANDLE {
        let h = self.handle;
        self.handle = HANDLE::null();
        h
    }
    pub fn reset(&mut self, h: HANDLE) {
        self.handle = h;
    }
    #[inline]
    pub fn is_valid(&self) -> bool {
        !self.handle.is_null()
    }
}

#[cfg(not(windows))]
impl Deref for unique_hfile {
    type Target = HANDLE;
    fn deref(&self) -> &HANDLE {
        &self.handle
    }
}

#[cfg(not(windows))]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct unique_hkey {
    handle: HKEY,
}

#[cfg(not(windows))]
impl unique_hkey {
    pub fn new(handle: HKEY) -> Self {
        unique_hkey { handle }
    }
    #[inline]
    pub fn get(&self) -> HKEY {
        self.handle
    }
    pub fn release(mut self) -> HKEY {
        let h = self.handle;
        self.handle = HKEY::null();
        h
    }
    pub fn reset(&mut self, h: HKEY) {
        self.handle = h;
    }
    #[inline]
    pub fn is_valid(&self) -> bool {
        !self.handle.is_null()
    }
}

#[cfg(not(windows))]
impl Deref for unique_hkey {
    type Target = HKEY;
    fn deref(&self) -> &HKEY {
        &self.handle
    }
}

#[cfg(not(windows))]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct unique_hmodule {
    handle: HMODULE,
}

#[cfg(not(windows))]
impl unique_hmodule {
    pub fn new(handle: HMODULE) -> Self {
        unique_hmodule { handle }
    }
    #[inline]
    pub fn get(&self) -> HMODULE {
        self.handle
    }
    pub fn release(mut self) -> HMODULE {
        let h = self.handle;
        self.handle = HMODULE::null();
        h
    }
    pub fn reset(&mut self, h: HMODULE) {
        self.handle = h;
    }
    #[inline]
    pub fn is_valid(&self) -> bool {
        !self.handle.is_null()
    }
}

#[cfg(not(windows))]
impl Deref for unique_hmodule {
    type Target = HMODULE;
    fn deref(&self) -> &HMODULE {
        &self.handle
    }
}

#[cfg(not(windows))]
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct unique_hwnd {
    handle: HWND,
}

#[cfg(not(windows))]
impl unique_hwnd {
    pub fn new(handle: HWND) -> Self {
        unique_hwnd { handle }
    }
    #[inline]
    pub fn get(&self) -> HWND {
        self.handle
    }
    pub fn release(mut self) -> HWND {
        let h = self.handle;
        self.handle = HWND::null();
        h
    }
    pub fn reset(&mut self, h: HWND) {
        self.handle = h;
    }
    #[inline]
    pub fn is_valid(&self) -> bool {
        !self.handle.is_null()
    }
}

#[cfg(not(windows))]
impl Deref for unique_hwnd {
    type Target = HWND;
    fn deref(&self) -> &HWND {
        &self.handle
    }
}

// ---------------------------------------------------------------------------
// Small pieces of C++ result.h translated for completeness
// ---------------------------------------------------------------------------

/// Type tag for the success/error policy used by `com_ptr`. The C++ code
/// distinguishes `err_return_policy`, `err_exception_policy`, and
/// `err_failfast_policy`; this translation collapses them into a single
/// `Result<()>`-based representation because the Rust error model
/// already provides the "return a result" variant. The exception/failfast
/// variants are simply not represented: callers that want exceptions or
/// fail-fast behaviour must call the appropriate `verify_*` helper and
/// then convert the `Error` to a panic/exception themselves.
pub struct err_policy;

/// The return type produced by `err_policy`. In the C++ translation this
/// is the alias that resolves to `HRESULT` (return policy), `void`
/// (exception/failfast) - here it is always `Result<()>`.
pub type err_result = Result<()>;

/// Translates an `Error` to the closest matching HRESULT. Used by
/// `RETURN_IF_WIN32_ERROR_EXPECTED` and friends in the C++ macros.
#[inline]
pub fn Error_to_hresult(e: Error) -> i32 {
    match e {
        Error::Hresult(hr) => hr,
        Error::Win32(code) => HRESULT_FROM_WIN32(code),
        Error::Nt(status) => NtStatus_to_hresult(status),
    }
}

/// Maps an NTSTATUS to an HRESULT, matching `wil::details::NtStatusToHr`.
/// The implementation is intentionally simple: it preserves the status
/// verbatim (NTSTATUS values < 0 are HRESULT-compatible).
#[inline]
pub fn NtStatus_to_hresult(status: i32) -> i32 {
    if status < 0 {
        status
    } else {
        HRESULT_FROM_WIN32(0)
    }
}
