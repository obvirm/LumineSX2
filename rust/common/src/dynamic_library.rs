// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! `dynamic_library` — RAII wrapper around dynamic library loading.
//!
//! Rust port of PCSX2's C++ `common/DynamicLibrary.{h,cpp}`.
//!
//! Uses [`libloading`] crate — cross-platform, wraps `dlopen`/`LoadLibrary`.

use core::ffi::{c_char, c_void};
use core::ptr;

// ---------------------------------------------------------------------------
// Pure-Rust surface
// ---------------------------------------------------------------------------

/// RAII handle to a dynamically-loaded shared library.
///
/// Wraps [`libloading::Library`]. Dropping unloads the library.
pub struct DynamicLibrary {
    inner: Option<libloading::Library>,
}

unsafe impl Send for DynamicLibrary {}

impl DynamicLibrary {
    /// Build a closed `DynamicLibrary`.
    #[inline]
    pub const fn new() -> Self {
        Self { inner: None }
    }

    /// Returns `true` if a library is loaded.
    #[inline]
    pub fn is_open(&self) -> bool {
        self.inner.is_some()
    }

    /// Load a shared library by filename.
    ///
    /// On Unix `filename` is passed to `dlopen`. On Windows it's the
    /// DLL path (`libloading` widens to UTF-16 internally).
    pub fn load(filename: &str) -> Result<Self, String> {
        // SAFETY: libloading::Library::new is unsafe because loading
        // a library runs its initialisation code. This matches the
        // C++ caller's responsibility when calling Open().
        match unsafe { libloading::Library::new(filename) } {
            Ok(lib) => Ok(Self { inner: Some(lib) }),
            Err(e) => Err(format!("loading '{filename}': {e}")),
        }
    }

    /// Close the library. Idempotent.
    pub fn close(&mut self) {
        self.inner = None; // Drop runs dlclose/FreeLibrary
    }

    /// Resolve a symbol address by name.
    ///
    /// Returns null if the symbol is not found or library is closed.
    ///
    /// # Safety
    ///
    /// The caller must cast the result to the correct type.
    pub unsafe fn get_symbol_address(&self, name: &str) -> *mut c_void {
        let Some(lib) = &self.inner else {
            return ptr::null_mut();
        };
        let mut cname: Vec<u8> = Vec::with_capacity(name.len() + 1);
        cname.extend_from_slice(name.as_bytes());
        cname.push(0);
        match unsafe { lib.get::<*mut c_void>(&cname) } {
            Ok(sym) => *sym,
            Err(_) => ptr::null_mut(),
        }
    }
}

impl Default for DynamicLibrary {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for DynamicLibrary {
    fn drop(&mut self) {
        self.close();
    }
}

// ---------------------------------------------------------------------------
// Utility: platform suffix helpers
// ---------------------------------------------------------------------------

/// Platform-specific shared library suffix.
#[inline]
pub fn platform_lib_suffix() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        ".dll"
    }
    #[cfg(target_os = "macos")]
    {
        ".dylib"
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        ".so"
    }
}

/// Add the platform suffix (GetUnprefixedFilename equivalent).
pub fn add_lib_suffix(filename: &str) -> String {
    format!("{}{}", filename, platform_lib_suffix())
}

/// Build a versioned library filename (GetVersionedFilename equivalent).
#[allow(unused_variables)]
pub fn versioned_filename(libname: &str, major: i32, minor: i32) -> String {
    #[cfg(target_os = "windows")]
    {
        if major >= 0 && minor >= 0 {
            format!("{}-{}-{}.dll", libname, major, minor)
        } else if major >= 0 {
            format!("{}-{}.dll", libname, major)
        } else {
            format!("{}.dll", libname)
        }
    }
    #[cfg(target_os = "macos")]
    {
        let prefix = if libname.starts_with("lib") { "" } else { "lib" };
        if major >= 0 && minor >= 0 {
            format!("{}{}.{}.{}.dylib", prefix, libname, major, minor)
        } else if major >= 0 {
            format!("{}{}.{}.dylib", prefix, libname, major)
        } else {
            format!("{}{}.dylib", prefix, libname)
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let prefix = if libname.starts_with("lib") { "" } else { "lib" };
        if major >= 0 && minor >= 0 {
            format!("{}{}.so.{}.{}", prefix, libname, major, minor)
        } else if major >= 0 {
            format!("{}{}.so.{}", prefix, libname, major)
        } else {
            format!("{}{}.so", prefix, libname)
        }
    }
}

// ---------------------------------------------------------------------------
// FFI surface
// ---------------------------------------------------------------------------

/// Load a shared library. Returns opaque handle or null.
/// Handle must be released with `pcsx2_dynlib_destroy`.
#[no_mangle]
pub extern "C" fn pcsx2_dynlib_load(name: *const c_char) -> *mut DynamicLibrary {
    if name.is_null() {
        return ptr::null_mut();
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(name) };
    let name_str = match cstr.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    match DynamicLibrary::load(name_str) {
        Ok(lib) => Box::into_raw(Box::new(lib)),
        Err(_) => ptr::null_mut(),
    }
}

/// Resolve a symbol from a loaded library.
/// Returns symbol address or null. Pointer borrowed — invalidated
/// when library is destroyed.
///
/// # Safety
///
/// `lib` must be valid (from `pcsx2_dynlib_load`). `name` must be a
/// valid C string. Caller casts the result to the correct type.
#[no_mangle]
pub extern "C" fn pcsx2_dynlib_get_symbol(
    lib: *mut DynamicLibrary,
    name: *const c_char,
) -> *mut c_void {
    if lib.is_null() || name.is_null() {
        return ptr::null_mut();
    }
    let lib_ref = unsafe { &*lib };
    let cstr = unsafe { std::ffi::CStr::from_ptr(name) };
    let name_str = match cstr.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };
    unsafe { lib_ref.get_symbol_address(name_str) }
}

/// Destroy a library handle (close + free).
///
/// # Safety
///
/// `lib` must be from `pcsx2_dynlib_load` or null (no-op).
#[no_mangle]
pub extern "C" fn pcsx2_dynlib_destroy(lib: *mut DynamicLibrary) {
    if lib.is_null() {
        return;
    }
    unsafe {
        let _ = Box::from_raw(lib);
    }
}
