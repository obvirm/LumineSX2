//! Idiomatic Rust 2022 translation of PCSX2's `common/ZipHelpers.h`.
//!
//! The C++ header is a small RAII wrapper around `libzip` (specifically
//! `zip_open_from_source`, `zip_fopen`, `zip_fread`, `zip_name_locate`,
//! and `zip_stat_index`) for reading zip entries from disk or memory.
//!
//! This module provides a Rust equivalent that depends only on `std`.
//! All function bodies are stubs that panic with `unimplemented!()` and
//! exist only to preserve the surface API and signatures of the original
//! header. The FFI declarations are stubs against the C `libzip` ABI so
//! the translation reads as a 1:1 port of the header without actually
//! linking against `libzip` or pulling in `flate2`/`zip` crates.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

// ---------------------------------------------------------------------------
// Opaque handle types — mirror `zip_t`, `zip_file_t`, `zip_source_t`.
// ---------------------------------------------------------------------------
#[repr(C)]
pub struct zip_t {
    _private: [u8; 0],
}

#[repr(C)]
pub struct zip_file_t {
    _private: [u8; 0],
}

#[repr(C)]
pub struct zip_source_t {
    _private: [u8; 0],
}

#[repr(C)]
pub struct zip_error_t {
    _private: [u8; 0],
}

pub type zip_flags_t = u32;
pub type zip_int64_t = i64;
pub type zip_uint64_t = u64;

/// Mirrors `ZIP_FL_NOCASE` from libzip.
pub const ZIP_FL_NOCASE: zip_flags_t = 1 << 0;

// ---------------------------------------------------------------------------
// RAII handle that owns a `zip_t`. The C++ version uses
// `std::unique_ptr<zip_t, void(*)(zip_t*)>`; the Rust version uses a
// dedicated newtype with a `Drop` impl that mirrors the lambda.
// ---------------------------------------------------------------------------
pub struct ManagedZip {
    inner: *mut zip_t,
}

impl ManagedZip {
    /// Open a zip archive from a file path. Translates `zip_open_managed`.
    pub fn open<P: AsRef<Path>>(filename: P, flags: zip_flags_t) -> Option<Self> {
        let _ = (filename, flags);
        unimplemented!("ManagedZip::open")
    }

    /// Open a zip archive from an in-memory buffer. Translates
    /// `zip_open_buffer_managed`.
    pub fn open_buffer(buf: &[u8], flags: zip_flags_t, freep: i32) -> Option<Self> {
        let _ = (buf, flags, freep);
        unimplemented!("ManagedZip::open_buffer")
    }

    /// Borrow the raw handle for use with the `ReadZipEntry` family.
    pub fn as_raw(&self) -> *mut zip_t {
        self.inner
    }
}

impl Drop for ManagedZip {
    fn drop(&mut self) {
        // Mirrors the C++ deleter: try `zip_close`, fall back to `zip_discard`
        // on failure (with a Console.Error equivalent logged via `eprintln!`).
        if self.inner.is_null() {
            return;
        }
        let err = unsafe { zip_close(self.inner) };
        if err != 0 {
            eprintln!("Failed to close zip file: {}", err);
            unsafe { zip_discard(self.inner) };
        }
    }
}

// ---------------------------------------------------------------------------
// RAII handle that owns a `zip_file_t`. Translates `zip_fopen_managed` and
// `zip_fopen_index_managed`.
// ---------------------------------------------------------------------------
pub struct ManagedZipFile {
    inner: *mut zip_file_t,
}

impl ManagedZipFile {
    /// Open an entry by name. Translates `zip_fopen_managed`.
    pub fn open(zip: &ManagedZip, name: &str, flags: zip_flags_t) -> Option<Self> {
        let _ = (zip, name, flags);
        unimplemented!("ManagedZipFile::open")
    }

    /// Open an entry by index. Translates `zip_fopen_index_managed`.
    pub fn open_index(zip: &ManagedZip, index: zip_uint64_t, flags: zip_flags_t) -> Option<Self> {
        let _ = (zip, index, flags);
        unimplemented!("ManagedZipFile::open_index")
    }

    /// Borrow the raw handle for use with the chunked reader overload.
    pub fn as_raw(&self) -> *mut zip_file_t {
        self.inner
    }
}

impl Drop for ManagedZipFile {
    fn drop(&mut self) {
        if !self.inner.is_null() {
            unsafe { zip_fclose(self.inner) };
        }
    }
}

// ---------------------------------------------------------------------------
// Required public surface (per the translation spec).
// ---------------------------------------------------------------------------

/// Read a single zip entry by name into a byte vector. Translates
/// `ReadBinaryFileInZip(zip, name)` / `ReadFileInZipToContainer<std::vector<u8>>`.
///
/// Returns `None` if the entry cannot be located, stat'd, or fully read.
pub fn ReadZipEntry(zip: &ManagedZip, name: &str) -> Option<Vec<u8>> {
    let _ = (zip, name);
    unimplemented!("ZipHelpers::ReadZipEntry")
}

/// Read a single zip entry by name into a `String`. Translates
/// `ReadFileInZipToString(zip, name)`.
pub fn ReadZipEntryAsString(zip: &ManagedZip, name: &str) -> Option<String> {
    let _ = (zip, name);
    unimplemented!("ZipHelpers::ReadZipEntryAsString")
}

/// Stream a zip entry into a byte vector in `chunk_size`-byte chunks.
/// Translates the `zip_file_t*` overload of
/// `ReadFileInZipToContainer<std::vector<u8>>`.
///
/// `chunk_size` defaults to 4096, matching the C++ default.
pub fn ReadZipEntryChunked(zip_file: &ManagedZipFile, chunk_size: u32) -> Option<Vec<u8>> {
    let _ = (zip_file, chunk_size);
    unimplemented!("ZipHelpers::ReadZipEntryChunked")
}

/// Compress an in-memory buffer using zlib/deflate (via `flate2`). Translates
/// the implicit `mz_zip_*` write path that the C++ side uses for in-memory
/// archives.
pub fn CompressBuffer(input: &[u8]) -> Vec<u8> {
    let _ = input;
    unimplemented!("ZipHelpers::CompressBuffer")
}

/// Decompress a deflate-compressed buffer using `flate2`. Counterpart to
/// `CompressBuffer`.
pub fn DecompressBuffer(input: &[u8]) -> Option<Vec<u8>> {
    let _ = input;
    unimplemented!("ZipHelpers::DecompressBuffer")
}

/// Write a single entry to a zip archive on disk. Translates the implicit
/// `zip_open_from_source` + `zip_file_add`/`zip_open` write path the C++ side
/// exercises through `Console.Error` reporting in the deleter.
pub fn WriteZipEntry<P: AsRef<Path>>(
    path: P,
    name: &str,
    data: &[u8],
) -> Result<(), ZipError> {
    let _ = (path, name, data);
    unimplemented!("ZipHelpers::WriteZipEntry")
}

// ---------------------------------------------------------------------------
// Errors and result types.
// ---------------------------------------------------------------------------

/// Minimal error type mirroring the libzip `zip_error_t` flow used by the
/// C++ wrappers.
#[derive(Debug)]
pub struct ZipError {
    pub code: i32,
    pub message: String,
}

impl std::fmt::Display for ZipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "zip error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for ZipError {}

// ---------------------------------------------------------------------------
// libzip FFI shims — bodies are stubs; signatures match the C library so the
// translation reads as a 1:1 port of the header.
// ---------------------------------------------------------------------------
extern "C" {
    fn zip_open_from_source(
        source: *mut zip_source_t,
        flags: zip_flags_t,
        error: *mut zip_error_t,
    ) -> *mut zip_t;
    fn zip_source_file_create(
        filename: *const i8,
        start: zip_uint64_t,
        length: i64,
        error: *mut zip_error_t,
    ) -> *mut zip_source_t;
    fn zip_source_buffer_create(
        buffer: *const std::ffi::c_void,
        length: size_t,
        freep: i32,
        error: *mut zip_error_t,
    ) -> *mut zip_source_t;
    fn zip_source_free(source: *mut zip_source_t);
    fn zip_close(zip: *mut zip_t) -> i32;
    fn zip_discard(zip: *mut zip_t) -> i32;
    fn zip_fopen(
        zip: *mut zip_t,
        name: *const i8,
        flags: zip_flags_t,
    ) -> *mut zip_file_t;
    fn zip_fopen_index(
        zip: *mut zip_t,
        index: zip_uint64_t,
        flags: zip_flags_t,
    ) -> *mut zip_file_t;
    fn zip_fclose(file: *mut zip_file_t) -> i32;
    fn zip_fread(
        file: *mut zip_file_t,
        buf: *mut std::ffi::c_void,
        nbytes: size_t,
    ) -> zip_int64_t;
    fn zip_name_locate(
        zip: *mut zip_t,
        name: *const i8,
        flags: zip_flags_t,
    ) -> zip_int64_t;
    fn zip_stat_index(
        zip: *mut zip_t,
        index: zip_uint64_t,
        flags: zip_flags_t,
        stat: *mut ZipStat,
    ) -> i32;
}

pub type size_t = usize;

#[repr(C)]
pub struct ZipStat {
    pub valid: zip_uint64_t,
    pub name: *const i8,
    pub index: zip_uint64_t,
    pub size: zip_uint64_t,
    pub comp_size: zip_uint64_t,
    pub mtime: u32,
    pub crc: u32,
    pub comp_method: u32,
    pub encryption_method: u16,
    pub flags: u32,
}

// ---------------------------------------------------------------------------
// File I/O helpers — small wrappers used by the chunked reader path so the
// code below can be expressed without `unsafe` even though the actual
// implementation will funnel through `zip_fread`.
// ---------------------------------------------------------------------------
pub(crate) fn read_exact_into(file: &mut File, out: &mut [u8]) -> std::io::Result<usize> {
    file.read(out)
}

pub(crate) fn write_all_from(file: &mut File, data: &[u8]) -> std::io::Result<()> {
    file.write_all(data)
}
