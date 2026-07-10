//! PCSX2 CDVD (CD/DVD Drive Emulation) - Rust Implementation
//!
//! Replaces `pcsx2/CDVD/` C++ code with pure Rust.
//!
//! Supported formats:
//! - ISO (raw ISO9660)
//! - CHD (Compressed Hunks of Data) via `chd` crate
//! - CSO/CISO (Compressed ISO)
//! - Blockdump (PCSX2 debug format)

mod reader;
mod iso_reader;
mod chd_reader;
mod cso_reader;
mod blockdump_reader;

pub use reader::{CDVDReader, CDVDError};
pub use iso_reader::IsoReader;
pub use chd_reader::ChdReader;
pub use cso_reader::CsoReader;
pub use blockdump_reader::BlockdumpReader;

use std::ffi::{CStr, c_char, c_int};
use std::ptr;

// ============================================================================
// FFI Exports for C++ interop
// ============================================================================

/// Opaque handle to a CDVDReader instance
#[repr(C)]
pub struct CDVDReaderHandle {
    _private: [u8; 0],
}

/// Open a CDVD image file. Returns NULL on error.
/// 
/// # Safety
/// `path` must be a valid null-terminated C string.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_cdvd_open(path: *const c_char) -> *mut CDVDReaderHandle {
    if path.is_null() {
        return ptr::null_mut();
    }

    let path_str = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    // Auto-detect format based on extension
    let reader: Box<dyn CDVDReader> = if path_str.ends_with(".iso") || path_str.ends_with(".ISO") {
        match IsoReader::open(path_str) {
            Ok(r) => Box::new(r),
            Err(_) => return ptr::null_mut(),
        }
    } else if path_str.ends_with(".chd") || path_str.ends_with(".CHD") {
        match ChdReader::open(path_str) {
            Ok(r) => Box::new(r),
            Err(_) => return ptr::null_mut(),
        }
    } else if path_str.ends_with(".cso") || path_str.ends_with(".CSO") {
        match CsoReader::open(path_str) {
            Ok(r) => Box::new(r),
            Err(_) => return ptr::null_mut(),
        }
    } else {
        // Default to ISO
        match IsoReader::open(path_str) {
            Ok(r) => Box::new(r),
            Err(_) => return ptr::null_mut(),
        }
    };

    Box::into_raw(reader) as *mut CDVDReaderHandle
}

/// Close and free a CDVD reader.
///
/// # Safety
/// `handle` must have been returned by `pcsx2_cdvd_open` and not yet freed.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_cdvd_close(handle: *mut CDVDReaderHandle) {
    if !handle.is_null() {
        let _ = Box::from_raw(handle as *mut Box<dyn CDVDReader>);
    }
}

/// Read sectors from the disc.
///
/// # Safety
/// - `handle` must be valid
/// - `buffer` must point to at least `sector_count * 2048` bytes
#[no_mangle]
pub unsafe extern "C" fn pcsx2_cdvd_read_sectors(
    handle: *mut CDVDReaderHandle,
    lsn: u32,
    sector_count: u32,
    buffer: *mut u8,
) -> c_int {
    if handle.is_null() || buffer.is_null() {
        return -1;
    }

    let reader = &mut *(handle as *mut Box<dyn CDVDReader>);
    let buf_slice = std::slice::from_raw_parts_mut(buffer, (sector_count * 2048) as usize);

    match reader.read_sectors(lsn, buf_slice) {
        Ok(bytes) => bytes as c_int,
        Err(_) => -1,
    }
}

/// Get the total size of the disc in bytes.
///
/// # Safety
/// `handle` must be valid.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_cdvd_get_size(handle: *mut CDVDReaderHandle) -> u64 {
    if handle.is_null() {
        return 0;
    }

    let reader = &*(handle as *mut Box<dyn CDVDReader>);
    reader.get_size()
}

/// Get the sector count (size / 2048).
///
/// # Safety
/// `handle` must be valid.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_cdvd_get_sector_count(handle: *mut CDVDReaderHandle) -> u32 {
    if handle.is_null() {
        return 0;
    }

    let reader = &*(handle as *mut Box<dyn CDVDReader>);
    (reader.get_size() / 2048) as u32
}
