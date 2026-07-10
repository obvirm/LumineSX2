//! Platform-independent dynamic library loader.
//!
//! This module is a thin RAII wrapper around the platform's dynamic-link APIs:
//! `dlopen` / `dlsym` on Unix-like systems, and `LoadLibraryW` /
//! `GetProcAddress` on Windows. The handle is released automatically when a
//! [`DynamicLibrary`] is dropped, or earlier via [`DynamicLibrary::close`].
//!
//! Only the standard library is required; the FFI surface is declared inline
//! using `extern "C"` (Unix) or `extern "system"` (Windows) blocks rather than
//! pulling in a `libc` dependency.

use std::ffi::c_void;
use std::path::Path;
use std::ptr;

// ---------------------------------------------------------------------------
// Platform-specific FFI layer
// ---------------------------------------------------------------------------

#[cfg(unix)]
mod platform {
    use std::ffi::{CStr, CString};
    use std::os::raw::c_char;

    pub type Handle = *mut c_void;

    // RTLD_NOW is 2 on glibc/musl/macOS, but its value is implementation-defined,
    // so we hard-code it here to avoid taking a dependency on `libc`.
    const RTLD_NOW: i32 = 2;

    extern "C" {
        fn dlopen(filename: *const c_char, flag: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> i32;
        fn dlerror() -> *const c_char;
    }

    pub fn open(path: &str) -> Result<Handle, String> {
        let c_path =
            CString::new(path).map_err(|e| format!("DynamicLibrary: invalid path: {}", e))?;
        let handle = unsafe { dlopen(c_path.as_ptr(), RTLD_NOW) };
        if handle.is_null() {
            // Capture `dlerror()` immediately; subsequent libc calls may clobber it.
            let raw = unsafe { dlerror() };
            let msg = if raw.is_null() {
                "<UNKNOWN>".to_string()
            } else {
                unsafe { CStr::from_ptr(raw) }.to_string_lossy().into_owned()
            };
            return Err(format!("Loading {} failed: {}", path, msg));
        }
        Ok(handle)
    }

    pub fn symbol(handle: Handle, name: &str) -> *mut c_void {
        let c_name = CString::new(name).expect("DynamicLibrary: symbol name contained NUL");
        unsafe { dlsym(handle, c_name.as_ptr()) }
    }

    pub fn close(handle: Handle) {
        unsafe { dlclose(handle) };
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::{c_void, CString};
    use std::os::raw::c_char;

    pub type Handle = *mut c_void;

    type HMODULE = *mut c_void;
    type LPCWSTR = *const u16;
    type LPCSTR = *const c_char;

    extern "system" {
        fn LoadLibraryW(lpFileName: LPCWSTR) -> HMODULE;
        fn GetProcAddress(hModule: HMODULE, lpProcName: LPCSTR) -> *mut c_void;
        fn FreeLibrary(hLibModule: HMODULE) -> i32;
        fn GetLastError() -> u32;
    }

    pub fn open(path: &str) -> Result<Handle, String> {
        // LoadLibraryW takes a null-terminated UTF-16 string, so encode the
        // incoming UTF-8 path and append a NUL terminator.
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        if handle.is_null() {
            let err = unsafe { GetLastError() };
            return Err(format!("Loading {} failed: 0x{:08x}", path, err));
        }
        Ok(handle)
    }

    pub fn symbol(handle: Handle, name: &str) -> *mut c_void {
        let c_name = CString::new(name).expect("DynamicLibrary: symbol name contained NUL");
        unsafe { GetProcAddress(handle, c_name.as_ptr()) }
    }

    pub fn close(handle: Handle) {
        unsafe { FreeLibrary(handle) };
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// RAII handle to a dynamically loaded shared library.
///
/// Construct an empty handle with [`DynamicLibrary::new`], or load a library
/// with [`DynamicLibrary::open`]. The underlying handle is released when the
/// value is dropped or when [`DynamicLibrary::close`] is called. Lookup of
/// exported symbols is performed by [`DynamicLibrary::get_symbol_address`].
pub struct DynamicLibrary {
    handle: *mut c_void,
    is_opened: bool,
}

impl DynamicLibrary {
    /// Returns a new, empty handle that does not reference any loaded library.
    pub fn new() -> Self {
        Self {
            handle: ptr::null_mut(),
            is_opened: false,
        }
    }

    /// Returns the specified library name with the platform-specific suffix
    /// added.
    ///
    /// Mirrors `DynamicLibrary::GetUnprefixedFilename`. Appends `.dll` on
    /// Windows, `.dylib` on Apple platforms, and `.so` elsewhere.
    pub fn get_unprefixed_filename(filename: &str) -> String {
        #[cfg(windows)]
        {
            format!("{}.dll", filename)
        }
        #[cfg(target_os = "macos")]
        {
            format!("{}.dylib", filename)
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            format!("{}.so", filename)
        }
    }

    /// Returns the specified library name in platform-specific versioned format.
    ///
    /// Mirrors `DynamicLibrary::GetVersionedFilename`. If `libname` already
    /// begins with the `"lib"` prefix, the prefix is not added again. Pass
    /// `-1` for either version number to omit it.
    ///
    /// Format examples:
    /// - Windows: `LIBNAME-MAJOR-MINOR.dll`
    /// - Linux:   `libLIBNAME.so.MAJOR.MINOR`
    /// - macOS:   `libLIBNAME.MAJOR.MINOR.dylib`
    pub fn get_versioned_filename(libname: &str, major: i32, minor: i32) -> String {
        #[cfg(windows)]
        {
            if major >= 0 && minor >= 0 {
                format!("{}-{}-{}.dll", libname, major, minor)
            } else if major >= 0 {
                format!("{}-{}.dll", libname, major)
            } else {
                format!("{}.dll", libname)
            }
        }
        #[cfg(not(windows))]
        {
            let prefix: &str = if libname.starts_with("lib") { "" } else { "lib" };
            #[cfg(target_os = "macos")]
            {
                if major >= 0 && minor >= 0 {
                    format!("{}{}.{}.{}.dylib", prefix, libname, major, minor)
                } else if major >= 0 {
                    format!("{}{}.{}.dylib", prefix, libname, major)
                } else {
                    format!("{}{}.dylib", prefix, libname)
                }
            }
            #[cfg(all(unix, not(target_os = "macos")))]
            {
                if major >= 0 && minor >= 0 {
                    format!("{}{}.so.{}.{}", prefix, libname, major, minor)
                } else if major >= 0 {
                    format!("{}{}.so.{}", prefix, libname, major)
                } else {
                    format!("{}{}.so", prefix, libname)
                }
            }
        }
    }

    /// Loads the dynamic library at `path`.
    ///
    /// On Unix this calls `dlopen(path, RTLD_NOW)`; on Windows it calls
    /// `LoadLibraryW` after converting `path` from UTF-8 to UTF-16. Returns
    /// an error string describing the failure cause (`dlerror()` on Unix,
    /// `GetLastError()` on Windows) on failure.
    ///
    /// On macOS, when a direct load fails for a non-absolute path, the
    /// loader additionally probes `{bundle}/Contents/Frameworks/{path}`,
    /// mirroring the C++ CocoaTools-based fallback.
    pub fn open(path: &str) -> Result<Self, String> {
        match platform::open(path) {
            Ok(handle) => Ok(Self {
                handle,
                is_opened: true,
            }),
            Err(primary_err) => {
                #[cfg(target_os = "macos")]
                {
                    if let Some(handle) = macos_frameworks_fallback(path) {
                        return Ok(Self {
                            handle,
                            is_opened: true,
                        });
                    }
                }
                let _ = primary_err;
                Err(primary_err)
            }
        }
    }

    /// Returns true if a library is currently loaded on this handle.
    ///
    /// Mirrors `DynamicLibrary::IsOpen`.
    pub fn is_open(&self) -> bool {
        self.is_opened
    }

    /// Returns the opaque OS-specific handle to the loaded library, or
    /// null if no library is currently open.
    ///
    /// Mirrors `DynamicLibrary::GetHandle`.
    pub fn get_handle(&self) -> *mut c_void {
        self.handle
    }

    /// Adopts, or takes ownership of, an existing opened library handle.
    ///
    /// Any library currently open on this handle is closed first. The
    /// supplied `handle` must be non-null (matching the C++
    /// `pxAssertRel(handle, "Handle is valid")` debug assertion).
    ///
    /// Mirrors `DynamicLibrary::Adopt`.
    pub fn adopt(&mut self, handle: *mut c_void) {
        assert!(!handle.is_null(), "DynamicLibrary::adopt: handle is null");
        self.close();
        self.handle = handle;
        self.is_opened = true;
    }

    /// Tries to load `path`, but treats a missing file as success with `None`.
    ///
    /// If `path` does not refer to an existing file, returns `Ok(None)` without
    /// invoking the loader. Otherwise this behaves like [`Self::open`]: a
    /// successful load yields `Ok(Some(lib))`, and a loader-level failure
    /// (bad format, unresolved dependencies, ...) yields `Err(msg)`.
    pub fn open_optional(path: &str) -> Result<Option<Self>, String> {
        if !Path::new(path).exists() {
            return Ok(None);
        }
        Self::open(path).map(Some)
    }

    /// Resolves the symbol named `name` and reinterprets its address as `T`.
    ///
    /// `T` must be `Copy`; in practice this is intended for function pointers
    /// (`fn(...) -> ...`) and raw `*mut` / `*const` data pointers. Returns
    /// `None` if the handle is closed or the symbol cannot be found.
    pub fn get_symbol_address<T: Copy>(&self, name: &str) -> Option<T> {
        if !self.is_opened {
            return None;
        }
        let ptr = platform::symbol(self.handle, name);
        if ptr.is_null() {
            None
        } else {
            // The loader returns a raw pointer whose alignment may not match
            // `T`'s; `read_unaligned` performs a bitwise copy of `T`-sized
            // bytes, which is sound because `T: Copy` (and the pointed-to
            // data is a function or POD exported by the library).
            Some(unsafe { (ptr as *const T).read_unaligned() })
        }
    }

    /// Unloads the library, if one is currently open. After this call
    /// [`Self::get_symbol_address`] will return `None` and any function
    /// pointers previously obtained from this handle become invalid.
    pub fn close(&mut self) {
        if self.is_opened {
            platform::close(self.handle);
            self.handle = ptr::null_mut();
            self.is_opened = false;
        }
    }
}

impl Default for DynamicLibrary {
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
// macOS-specific helpers
// ---------------------------------------------------------------------------

/// On macOS, attempts to load `path` from inside the running app bundle's
/// `Contents/Frameworks/` directory after a direct `dlopen(path)` has failed.
///
/// This mirrors the C++ behavior at `common/DynamicLibrary.cpp:99-118`,
/// where a failed `dlopen` of a non-absolute path falls back to
/// `{bundle_path}/Contents/Frameworks/{path}`. Returns `None` if no
/// framework bundle path is available, the input is absolute, or the
/// candidate file does not exist or fails to load.
#[cfg(target_os = "macos")]
fn macos_frameworks_fallback(path: &str) -> Option<platform::Handle> {
    if Path::new(path).is_absolute() {
        return None;
    }
    let bundle_path = macos_bundle_path()?;
    let candidate = format!("{}/Contents/Frameworks/{}", bundle_path, path);
    if !Path::new(&candidate).exists() {
        return None;
    }
    platform::open(&candidate).ok()
}

/// Returns the on-disk path of the running macOS app bundle, or `None`
/// when the bundle path cannot be determined.
///
/// Mirrors `CocoaTools::GetBundlePath()` in the C++ source. The full
/// translation of `CocoaTools` is not yet available, so this currently
/// returns `None` and the Frameworks fallback in [`DynamicLibrary::open`]
/// is effectively a no-op. When `CocoaTools` is ported this should call
/// into the equivalent of `CFBundleGetMainBundle()` /
/// `CFBundleCopyExecutableURL()`.
#[cfg(target_os = "macos")]
fn macos_bundle_path() -> Option<String> {
    // TODO(common/CocoaTools): replace with `CocoaTools::get_bundle_path()`
    // once the CocoaTools Rust translation lands.
    let _ = std::iter::empty::<String>();
    None
}
