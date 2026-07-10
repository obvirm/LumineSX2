//! Cross-platform FFI shim for the host system query functions.

#![allow(clippy::missing_safety_doc)]

use crate::host_sys;

#[no_mangle]
pub extern "C" fn pcsx2_host_page_size() -> u32 {
    host_sys::get_runtime_page_size() as u32
}

#[no_mangle]
pub extern "C" fn pcsx2_host_cache_line_size() -> u32 {
    host_sys::get_runtime_cache_line_size() as u32
}

#[no_mangle]
pub extern "C" fn pcsx2_host_tick_frequency() -> u64 {
    host_sys::get_tick_frequency()
}

#[no_mangle]
pub extern "C" fn pcsx2_host_cpu_ticks() -> u64 {
    host_sys::get_cpu_ticks()
}

#[no_mangle]
pub extern "C" fn pcsx2_host_physical_memory() -> u64 {
    host_sys::get_physical_memory()
}

#[no_mangle]
pub extern "C" fn pcsx2_host_available_memory() -> u64 {
    host_sys::get_available_memory()
}

#[no_mangle]
pub extern "C" fn pcsx2_host_os_version_string(out: *mut u8, out_len: u32) -> u32 {
    use std::ffi::CString;
    if out.is_null() || out_len == 0 {
        return 0;
    }
    let s = host_sys::get_os_version_string();
    let cstr = match CString::new(s) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let bytes = cstr.as_bytes_with_nul();
    let needed = bytes.len() as u32;
    if needed <= out_len {
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), out, needed as usize);
        }
    }
    needed.saturating_sub(1)
}