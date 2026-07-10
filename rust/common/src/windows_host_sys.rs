// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Windows-specific host system operations — memory protection,
//! shared memory, and abort-with-message.
//!
//! This module provides the Win32 back-end for operations that are
//! NOT already covered by the cross-platform `host_sys.rs` + its
//! `#[cfg(windows)]` blocks. Functions duplicated there (page size,
//! ticks, physical memory, OS version string) are intentionally
//! omitted here to avoid linker conflicts.
//!
//! Unique contributions:
//! - `mem_protect` / `pcsx2_host_mem_protect` — `VirtualProtect` wrapper
//! - `create_shared_memory` / `destroy_shared_memory` — Win32 file-mapping
//! - `SharedMemoryMappingArea` — placeholder-based mapping region
//! - `abort_with_message` — terminate with diagnostic message

#![cfg(target_os = "windows")]
#![allow(dead_code, unused_imports, unused_variables, non_camel_case_types)]

use std::ffi::c_void;
use std::ptr;
use std::sync::{Mutex, MutexGuard, OnceLock};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Memory::{
    MapViewOfFile, MEMORY_MAPPED_VIEW_ADDRESS, UnmapViewOfFile, VirtualProtect,
    FILE_MAP_ALL_ACCESS,
    PAGE_EXECUTE_READ, PAGE_EXECUTE_READWRITE, PAGE_NOACCESS, PAGE_READONLY, PAGE_READWRITE,
};

// CreateFileMappingW lives in a separate feature in windows-sys 0.61.
// Declare it locally with the C ABI so we don't pull in extra features.
#[link(name = "kernel32")]
extern "system" {
    fn CreateFileMappingW(
        h_file: HANDLE,
        lp_file_mapping_attributes: *mut c_void,
        fl_protect: u32,
        dw_maximum_size_high: u32,
        dw_maximum_size_low: u32,
        lp_name: *const u16,
    ) -> HANDLE;
}

// ============================================================================
// Memory protection
// ============================================================================

/// Apply Win32 memory protection to a region of virtual memory.
///
/// `base` and `size` describe a region that must already be
/// allocated (typically via `VirtualAlloc`); this function only
/// changes the page protection bits. The `read` / `write` / `exec`
/// flags mirror the C++ `PageProtectionMode` helper.
///
/// Returns `true` on success; `false` on any Win32 failure.
pub fn mem_protect(base: *mut u8, size: usize, read: bool, write: bool, exec: bool) -> bool {
    let protect = if exec {
        if write {
            PAGE_EXECUTE_READWRITE
        } else {
            PAGE_EXECUTE_READ
        }
    } else if read {
        if write {
            PAGE_READWRITE
        } else {
            PAGE_READONLY
        }
    } else {
        PAGE_NOACCESS
    };

    let mut old_protect: u32 = 0;
    let ok = unsafe { VirtualProtect(base as *const c_void, size, protect, &mut old_protect) };
    ok != 0
}

// ============================================================================
// Shared memory (pagefile-backed file-mapping objects)
// ============================================================================

/// Side table mapping view base address → file-mapping handle.
fn shared_handles() -> MutexGuard<'static, HashMapWrapper> {
    static REGISTRY: OnceLock<Mutex<HashMapWrapper>> = OnceLock::new();
    REGISTRY
        .get_or_init(|| Mutex::new(HashMapWrapper::new()))
        .lock()
        .unwrap()
}

struct HashMapWrapper {
    inner: std::collections::HashMap<usize, usize>,
}

impl HashMapWrapper {
    fn new() -> Self {
        Self {
            inner: std::collections::HashMap::new(),
        }
    }

    fn insert(&mut self, key: usize, handle: HANDLE) {
        self.inner.insert(key, handle as usize);
    }

    fn remove(&mut self, key: &usize) -> Option<HANDLE> {
        self.inner.remove(key).map(|h| h as HANDLE)
    }
}

/// Create a pagefile-backed shared memory mapping of `size` bytes
/// and return a pointer to the mapped view.
pub fn create_shared_memory(name: &str, size: usize) -> *mut u8 {
    let wide_name: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let max_size_low = (size & 0xFFFF_FFFF) as u32;
    let max_size_high = (size >> 32) as u32;

    let handle = unsafe {
        CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            ptr::null_mut(),
            PAGE_READWRITE,
            max_size_high,
            max_size_low,
            wide_name.as_ptr(),
        )
    };

    if handle.is_null() {
        return ptr::null_mut();
    }

    let view = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, size) };
    if view.Value.is_null() {
        unsafe { CloseHandle(handle) };
        return ptr::null_mut();
    }

    let view_addr = view.Value as usize;
    shared_handles().insert(view_addr, handle);
    view.Value as *mut u8
}

/// Tear down a shared memory mapping created by [`create_shared_memory`].
pub fn destroy_shared_memory(ptr: *mut u8, _size: usize) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
            Value: ptr as *mut c_void,
        })
    };

    let key = ptr as usize;
    if let Some(handle) = shared_handles().remove(&key) {
        unsafe { CloseHandle(handle) };
    }
}

// ============================================================================
// Abort with message
// ============================================================================

/// Terminate the process with a diagnostic message.
pub fn abort_with_message(msg: &str) -> ! {
    eprintln!("PCSX2 fatal error: {}", msg);
    std::process::exit(1);
}

// ============================================================================
// Shared memory mapping area (placeholder)
// ============================================================================

/// Placeholder range reservation for fine-grained mapping management.
/// The C++ version is a stateful class with `Map` / `Unmap` methods
/// wrapping `VirtualAlloc2`, `MapViewOfFile3`, placeholder splitting
/// and coalescing. This is a minimal stub recording the base pointer
/// and size. A future revision can flesh out the full state machine.
pub struct SharedMemoryMappingArea {
    base_ptr: *mut u8,
    size: usize,
}

// ============================================================================
// FFI surface
// ============================================================================

/// FFI export: apply Win32 memory protection to a region.
///
/// `prot` is a bitfield compatible with the C++ `PageProtectionMode`:
/// - bit 0 (`0x1`): read
/// - bit 1 (`0x2`): write
/// - bit 2 (`0x4`): execute
///
/// Returns `true` on success.
#[no_mangle]
pub extern "C" fn pcsx2_host_mem_protect(base: *mut u8, size: usize, prot: u32) -> bool {
    let read = (prot & 0x1) != 0;
    let write = (prot & 0x2) != 0;
    let exec = (prot & 0x4) != 0;
    mem_protect(base, size, read, write, exec)
}
