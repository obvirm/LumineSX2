// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Pure-Rust port of `common/ZipHelpers.h`.
//
// Dependency (add to `Cargo.toml` `[dependencies]`):
//
// ```text
// zip = "0.6"
// ```
//
// The original C++ header is a thin RAII wrapper around libzip. The Rust
// `zip` crate already provides RAII handles (`ZipArchive`, `ZipFile`) via
// the `Drop` trait, so most of the original boilerplate disappears. The
// FFI surface here mirrors the C++ entry points that the PCSX2 core uses
// for save-state metadata, code patches, and game-data extraction.
//
// Note: the zip 0.6 API surfaces `ZipFile<'a>` as a separate `Read`-only
// handle borrowed from `ZipArchive`. The `read_to_end` call therefore
// requires `&mut ZipArchive` access, which is what the borrow checker
// was rejecting in the original (zip 0.5-style) port. We work around
// this by scoping the mutable borrow tightly around each entry access.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::Path;

use zip::result::ZipError;

/// Read the entire contents of the named entry inside `archive` into a
/// freshly allocated `Vec<u8>`.
pub fn read_file(archive_path: &Path, entry_name: &str) -> Result<Vec<u8>, Error> {
    let mut archive = open_archive(archive_path)?;

    // `by_name` returns `Result<ZipFile<'_>, _>` which borrows the
    // archive mutably for the duration of the returned handle. We do
    // the entire read in this scope, then drop the handle before the
    // archive goes out of scope at the function end.
    let mut buf = Vec::new();
    match archive.by_name(entry_name) {
        Ok(mut file) => {
            file.read_to_end(&mut buf).map_err(Error::Io)?;
        }
        Err(ZipError::FileNotFound) => return Ok(Vec::new()),
        Err(e) => return Err(Error::Zip(e)),
    }
    Ok(buf)
}

/// Enumerate the names of every file in `archive`.
pub fn list_files(archive_path: &Path) -> Result<Vec<String>, Error> {
    let archive = open_archive(archive_path)?;
    Ok(archive.file_names().map(str::to_owned).collect())
}

/// Extract every entry in `archive` into `dest_dir`.
pub fn extract_zip(archive_path: &Path, dest_dir: &Path) -> Result<(), Error> {
    let mut archive = open_archive(archive_path)?;
    fs::create_dir_all(dest_dir).map_err(Error::Io)?;

    let len = archive.len();
    for i in 0..len {
        // Each iteration opens and immediately closes its own borrow
        // scope, so we can re-borrow `archive` mutably next iteration.
        let entry_path = match archive.by_index(i) {
            Ok(entry) => {
                let safe_name = match entry.enclosed_name() {
                    Some(p) => p.to_path_buf(),
                    None => continue, // skip unsafe paths rather than abort
                };
                let is_dir = entry.is_dir();
                let out_path = dest_dir.join(&safe_name);
                if is_dir {
                    fs::create_dir_all(&out_path).map_err(Error::Io)?;
                } else {
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent).map_err(Error::Io)?;
                    }
                    let mut out = File::create(&out_path).map_err(Error::Io)?;
                    let mut entry = entry; // make mutable for io::copy
                    io::copy(&mut entry, &mut out).map_err(Error::Io)?;
                }
                safe_name
            }
            Err(_) => continue,
        };
        let _ = entry_path;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Unified error type for this module.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),
    Zip(ZipError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "zip helper I/O error: {e}"),
            Error::Zip(e) => write!(f, "zip helper archive error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            Error::Zip(e) => Some(e),
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::Io(e)
    }
}

fn open_archive(path: &Path) -> Result<zip::ZipArchive<File>, Error> {
    let file = File::open(path).map_err(Error::Io)?;
    zip::ZipArchive::new(file).map_err(Error::Zip)
}

// ===========================================================================
// FFI surface
// ===========================================================================

#[no_mangle]
pub extern "C" fn pcsx2_zip_extract(
    archive: *const std::os::raw::c_char,
    dest_dir: *const std::os::raw::c_char,
) -> bool {
    if archive.is_null() || dest_dir.is_null() {
        return false;
    }
    let archive_path = unsafe { cstr_to_path(archive) };
    let dest_path = unsafe { cstr_to_path(dest_dir) };
    match (archive_path, dest_path) {
        (Some(a), Some(d)) => match extract_zip(&a, &d) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("pcsx2_zip_extract failed: {e}");
                false
            }
        },
        _ => false,
    }
}

#[no_mangle]
pub extern "C" fn pcsx2_zip_list(
    archive: *const std::os::raw::c_char,
    out_count: *mut u32,
    out_names: *mut *mut *mut std::os::raw::c_char,
) -> bool {
    if archive.is_null() || out_count.is_null() || out_names.is_null() {
        return false;
    }
    let archive_path = unsafe { cstr_to_path(archive) };
    let Some(archive_path) = archive_path else {
        return false;
    };

    let names = match list_files(&archive_path) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("pcsx2_zip_list failed: {e}");
            return false;
        }
    };

    let count = names.len() as u32;
    let layout = std::alloc::Layout::array::<*mut std::os::raw::c_char>(count as usize + 1)
        .expect("zip name array layout overflow");
    let raw = unsafe { std::alloc::alloc(layout) as *mut *mut std::os::raw::c_char };
    if raw.is_null() {
        return false;
    }

    for (i, name) in names.iter().enumerate() {
        let bytes = name.as_bytes();
        let layout_s = std::alloc::Layout::array::<u8>(bytes.len() + 1)
            .expect("zip name string layout overflow");
        let s = unsafe { std::alloc::alloc(layout_s) as *mut std::os::raw::c_char };
        if s.is_null() {
            for j in 0..i {
                let p = unsafe { *raw.add(j) as *mut u8 };
                let len = unsafe { libc_strlen(p) } + 1;
                let l = std::alloc::Layout::array::<u8>(len).unwrap();
                unsafe { std::alloc::dealloc(p, l) };
            }
            unsafe { std::alloc::dealloc(raw as *mut u8, layout) };
            return false;
        }
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), s as *mut u8, bytes.len());
            *s.add(bytes.len()) = 0;
        }
        unsafe {
            *raw.add(i) = s;
        }
    }
    unsafe {
        *raw.add(count as usize) = std::ptr::null_mut();
        *out_count = count;
        *out_names = raw;
    }
    true
}

unsafe fn cstr_to_path(p: *const std::os::raw::c_char) -> Option<std::path::PathBuf> {
    let cstr = unsafe { std::ffi::CStr::from_ptr(p) };
    std::str::from_utf8(cstr.to_bytes())
        .ok()
        .map(std::path::PathBuf::from)
}

unsafe fn libc_strlen(p: *const u8) -> usize {
    unsafe {
        let mut n = 0;
        while *p.add(n) != 0 {
            n += 1;
        }
        n
    }
}
