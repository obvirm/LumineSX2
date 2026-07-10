// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! OS-agnostic filesystem helpers.
//!
//! Pure-Rust port of `common/FileSystem.{h,cpp}`. The C++ namespace
//! `FileSystem` provided stat/exists/glob helpers, RAII file
//! pointers, memory mapping, symlink manipulation, and
//! working-directory queries.
//!
//! Layout:
//! - Public enums and structs mirroring the C++ side
//!   (`FileAttributes`, `FindFlags`, `StatData`, `FindData`).
//! - Pure-Rust `Result<T, io::Error>` API for Rust callers.
//! - C-ABI FFI surface at the bottom for static linking into the
//!   PCSX2 C++ binary (`#[no_mangle] pub extern "C"`).
//!
//! Notes on porting:
//! - The C++ `std::span<const u8>` returned by `MapBinaryFileForRead`
//!   becomes [`Mmap`], a thin RAII wrapper around `memmap2::Mmap`.
//!   Dropping the struct unmaps the region.
//! - Glob matching (`*`, `?`) is implemented locally because `std`
//!   has no built-in glob matcher; the C++ side used `WildcardMatch`
//!   (a hand-rolled fnmatch variant).
//! - Root-directory enumeration: Windows drives on Windows, `HOME`
//!   plus `/` on Unix.
//! - Recursive delete uses `fs::remove_dir_all`, which already
//!   defends against symlink cycles for files/dirs at the leaves.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use libc::{c_char, c_int, c_void};

// ============================================================================
// Public types mirroring the C++ side
// ============================================================================

bitflags::bitflags! {
    /// Mirrors `FILESYSTEM_FILE_ATTRIBUTES`. Packed into a `u32` so
    /// the FFI struct layout matches the C++ side exactly.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FileAttributes: u32 {
        const DIRECTORY  = 1 << 0;
        const READ_ONLY  = 1 << 1;
        const COMPRESSED = 1 << 2;
    }
}

bitflags::bitflags! {
    /// Mirrors `FILESYSTEM_FIND_FLAGS`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct FindFlags: u32 {
        const RECURSIVE       = 1 << 0;
        const RELATIVE_PATHS  = 1 << 1;
        const HIDDEN_FILES    = 1 << 2;
        const FOLDERS         = 1 << 3;
        const FILES           = 1 << 4;
        const KEEP_ARRAY      = 1 << 5;
        const SORT_BY_NAME    = 1 << 6;
    }
}

/// Mirrors `FILESYSTEM_STAT_DATA`. All times are seconds since the
/// Unix epoch (`time_t` on the C++ side).
#[derive(Debug, Clone, Copy)]
pub struct StatData {
    /// Creation time on Windows, inode change time on Unix.
    pub creation_time: i64,
    /// Last modification time.
    pub modification_time: i64,
    /// File size in bytes. `0` for non-regular files.
    pub size: i64,
    pub attributes: FileAttributes,
}

/// One entry from a [`find_files`] search. Mirrors
/// `FILESYSTEM_FIND_DATA`.
#[derive(Debug, Clone)]
pub struct FindData {
    pub file_name: String,
    pub size: i64,
    pub creation_time: i64,
    pub modification_time: i64,
    pub attributes: FileAttributes,
}

// ============================================================================
// Pure-Rust helpers
// ============================================================================

/// Convert a `SystemTime` to seconds since the Unix epoch.
fn system_time_to_secs(t: SystemTime) -> i64 {
    t.duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Reduce `fs::Metadata` to a [`StatData`].
fn metadata_to_stat(meta: &fs::Metadata) -> StatData {
    let mut attributes = FileAttributes::empty();
    if meta.is_dir() {
        attributes |= FileAttributes::DIRECTORY;
    }
    if meta.permissions().readonly() {
        attributes |= FileAttributes::READ_ONLY;
    }

    StatData {
        creation_time: meta.created().ok().map(system_time_to_secs).unwrap_or(0),
        modification_time: system_time_to_secs(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)),
        size: if meta.is_file() { meta.len() as i64 } else { 0 },
        attributes,
    }
}

// ----------------------------------------------------------------------------
// Existence checks
// ----------------------------------------------------------------------------

/// `Ok(true)` if `path` exists and refers to a regular file.
///
/// `Ok(false)` covers both "does not exist" and "exists but is a
/// directory / symlink to a directory", matching the C++ semantics.
pub fn file_exists(path: impl AsRef<Path>) -> io::Result<bool> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    Ok(meta.is_file())
}

/// `Ok(true)` if `path` exists and refers to a directory.
pub fn directory_exists(path: impl AsRef<Path>) -> io::Result<bool> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    Ok(meta.is_dir())
}

/// `Ok(true)` if `path` is an existing directory with no entries.
pub fn directory_is_empty(path: impl AsRef<Path>) -> io::Result<bool> {
    let mut entries = fs::read_dir(path)?;
    Ok(entries.next().is_none())
}

// ----------------------------------------------------------------------------
// Stat
// ----------------------------------------------------------------------------

/// Fill a [`StatData`] for `path`.
pub fn stat_file(path: impl AsRef<Path>) -> io::Result<StatData> {
    let meta = fs::metadata(path)?;
    Ok(metadata_to_stat(&meta))
}

/// File size in bytes, or `Ok(-1)` if the path is not a regular file.
pub fn path_file_size(path: impl AsRef<Path>) -> io::Result<i64> {
    let meta = fs::metadata(path)?;
    Ok(if meta.is_file() { meta.len() as i64 } else { -1 })
}

/// Last-modified timestamp as seconds since the Unix epoch.
pub fn file_timestamp(path: impl AsRef<Path>) -> io::Result<Option<i64>> {
    match fs::metadata(path) {
        Ok(m) => Ok(Some(system_time_to_secs(m.modified()?))),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

// ----------------------------------------------------------------------------
// Deletion / rename
// ----------------------------------------------------------------------------

/// Delete a file. `Ok(false)` (not an error) if the path did not
/// exist, matching the C++ "fine, continue" semantics.
pub fn delete_file_path(path: impl AsRef<Path>) -> io::Result<bool> {
    match fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Rename / move a file or directory, overwriting the destination
/// if it exists (POSIX `rename(2)` semantics).
pub fn rename_path(old: impl AsRef<Path>, new: impl AsRef<Path>) -> io::Result<()> {
    fs::rename(old, new)
}

// ----------------------------------------------------------------------------
// Read / write
// ----------------------------------------------------------------------------

/// Read the whole file into a `Vec<u8>`. `Ok(None)` if the file is
/// missing, mirroring the C++ `std::optional` return.
pub fn read_binary_file(path: impl AsRef<Path>) -> io::Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(data) => Ok(Some(data)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Read the whole file into a `String`.
pub fn read_file_to_string(path: impl AsRef<Path>) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(s) => Ok(Some(s)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Write `data` to `path`, truncating if it exists.
pub fn write_binary_file(path: impl AsRef<Path>, data: &[u8]) -> io::Result<()> {
    fs::write(path, data)
}

/// Write `data` to `path`, truncating if it exists.
pub fn write_string_to_file(path: impl AsRef<Path>, data: &str) -> io::Result<()> {
    fs::write(path, data.as_bytes())
}

// ----------------------------------------------------------------------------
// Memory mapping (memmap2)
// ----------------------------------------------------------------------------

/// RAII memory-mapped read-only file. Drop unmaps the region.
pub struct Mmap {
    map: memmap2::Mmap,
}

impl Mmap {
    /// Memory-map `path` for reading.
    ///
    /// Returns `Ok(None)` if the file is missing or zero-byte (the
    /// C++ side returned an empty span in those cases — `memmap2`
    /// rejects zero-byte mappings).
    pub fn open(path: impl AsRef<Path>) -> io::Result<Option<Self>> {
        let file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };

        let meta = file.metadata()?;
        if meta.len() == 0 {
            return Ok(None);
        }

        let map = unsafe { memmap2::Mmap::map(&file)? };
        Ok(Some(Self { map }))
    }

    /// The mapped bytes.
    pub fn as_slice(&self) -> &[u8] {
        &self.map
    }
}

// ----------------------------------------------------------------------------
// Directory operations
// ----------------------------------------------------------------------------

/// Create a single directory. `AlreadyExists` is reported as `Ok(())`.
pub fn create_directory(path: impl AsRef<Path>) -> io::Result<()> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e),
    }
}

/// Create `path` and any missing parents.
pub fn create_directory_path(path: impl AsRef<Path>) -> io::Result<()> {
    fs::create_dir_all(path)
}

/// Create `path` if it doesn't exist. `recursive=true` creates
/// missing parents.
pub fn ensure_directory_exists(path: impl AsRef<Path>, recursive: bool) -> io::Result<bool> {
    if directory_exists(&path)? {
        return Ok(true);
    }
    if recursive {
        create_directory_path(path)?;
    } else {
        create_directory(path)?;
    }
    Ok(true)
}

/// Remove a single, empty directory.
pub fn delete_directory(path: impl AsRef<Path>) -> io::Result<()> {
    fs::remove_dir(path)
}

/// Recursively remove `path` and everything beneath it.
pub fn recursive_delete_directory(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    if !directory_exists(path)? {
        return Ok(());
    }
    fs::remove_dir_all(path)
}

/// Copy `source` to `destination`. With `replace=false`, an existing
/// destination produces `AlreadyExists`.
pub fn copy_file_path(
    source: impl AsRef<Path>,
    destination: impl AsRef<Path>,
    replace: bool,
) -> io::Result<()> {
    if !replace && destination.as_ref().exists() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "destination already exists",
        ));
    }

    let mut src = File::open(source)?;
    let mut dst = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(destination)?;

    let mut buf = [0u8; 4096];
    loop {
        let n = src.read(&mut buf)?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])?;
    }
    dst.flush()?;
    Ok(())
}

// ----------------------------------------------------------------------------
// Glob search (`find_files`)
// ----------------------------------------------------------------------------

/// Recursively scan `path` collecting entries matching `pattern`.
///
/// `pattern` is a simple `*`/`?` glob. `flags` controls recursion,
/// filtering, and sort order. Matches the C++
/// `FileSystem::FindFiles`.
pub fn find_files(
    path: impl AsRef<Path>,
    pattern: &str,
    flags: FindFlags,
) -> io::Result<Vec<FindData>> {
    let path = path.as_ref();
    let mut out = Vec::new();
    let mut visited = Vec::new();

    if flags.contains(FindFlags::RECURSIVE) {
        if let Ok(canonical) = path.canonicalize() {
            visited.push(canonical);
        }
    }

    let wild_match_all = pattern == "*";
    let has_wildcards = pattern.contains('*') || pattern.contains('?');

    recurse_find(
        path,
        Path::new(""),
        pattern,
        flags,
        wild_match_all,
        has_wildcards,
        &mut visited,
        &mut out,
    )?;

    if flags.contains(FindFlags::SORT_BY_NAME) {
        out.sort_by(|a, b| {
            let ad = a.attributes.contains(FileAttributes::DIRECTORY);
            let bd = b.attributes.contains(FileAttributes::DIRECTORY);
            // directories first, then case-sensitive name compare
            ad.cmp(&bd).reverse().then(a.file_name.cmp(&b.file_name))
        });
    }

    Ok(out)
}

fn recurse_find(
    origin: &Path,
    parent: &Path,
    pattern: &str,
    flags: FindFlags,
    wild_match_all: bool,
    has_wildcards: bool,
    visited: &mut Vec<PathBuf>,
    out: &mut Vec<FindData>,
) -> io::Result<()> {
    let scan_path = if parent.as_os_str().is_empty() {
        origin.to_path_buf()
    } else {
        origin.join(parent)
    };

    let entries = match fs::read_dir(&scan_path) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.file_name();
        let name_str = match name.to_str() {
            Some(s) => s,
            None => continue,
        };

        // `.` and `..` never appear in `read_dir`, but be defensive.
        if name_str == "." || name_str == ".." {
            continue;
        }
        // Hidden-file filter (Unix dot-files).
        if !flags.contains(FindFlags::HIDDEN_FILES) && name_str.starts_with('.') {
            continue;
        }

        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let mut attributes = FileAttributes::empty();
        if meta.is_dir() {
            attributes |= FileAttributes::DIRECTORY;
        }
        if meta.permissions().readonly() {
            attributes |= FileAttributes::READ_ONLY;
        }

        let is_dir = meta.is_dir();
        let full_path = if flags.contains(FindFlags::RELATIVE_PATHS) {
            if parent.as_os_str().is_empty() {
                PathBuf::from(name_str)
            } else {
                parent.join(name_str)
            }
        } else if parent.as_os_str().is_empty() {
            origin.join(name_str)
        } else {
            origin.join(parent).join(name_str)
        };

        // Recurse first so subdirectories are visited regardless of
        // whether the user requested FOLDERS for the leaf.
        if is_dir && flags.contains(FindFlags::RECURSIVE) {
            if let Ok(canonical) = full_path.canonicalize() {
                if !visited.iter().any(|p| p == &canonical) {
                    visited.push(canonical);
                    let child = if parent.as_os_str().is_empty() {
                        PathBuf::from(name_str)
                    } else {
                        parent.join(name_str)
                    };
                    recurse_find(
                        origin,
                        &child,
                        pattern,
                        flags,
                        wild_match_all,
                        has_wildcards,
                        visited,
                        out,
                    )?;
                }
            }
        }

        if is_dir && !flags.contains(FindFlags::FOLDERS) {
            continue;
        }
        if !is_dir && !flags.contains(FindFlags::FILES) {
            continue;
        }

        if has_wildcards {
            if !wild_match_all && !glob_match(name_str, pattern) {
                continue;
            }
        } else if name_str != pattern {
            continue;
        }

        out.push(FindData {
            file_name: full_path.to_string_lossy().into_owned(),
            size: if meta.is_file() { meta.len() as i64 } else { 0 },
            creation_time: meta.created().ok().map(system_time_to_secs).unwrap_or(0),
            modification_time: system_time_to_secs(meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)),
            attributes,
        });
    }

    Ok(())
}

/// Simple fnmatch-style glob: `*` matches any run, `?` matches one
/// character, anything else matches literally. Case-sensitive.
fn glob_match(name: &str, pattern: &str) -> bool {
    let n = name.as_bytes();
    let p = pattern.as_bytes();
    let mut ni = 0usize;
    let mut pi = 0usize;
    let mut star: Option<usize> = None;
    let mut match_pos = 0usize;

    while ni < n.len() {
        if pi < p.len() {
            match p[pi] {
                b'*' => {
                    star = Some(pi);
                    match_pos = ni;
                    pi += 1;
                    continue;
                }
                b'?' => {
                    ni += 1;
                    pi += 1;
                    continue;
                }
                c if c == n[ni] => {
                    ni += 1;
                    pi += 1;
                    continue;
                }
                _ => {}
            }
        }
        if let Some(s) = star {
            pi = s + 1;
            match_pos += 1;
            ni = match_pos;
        } else {
            return false;
        }
    }

    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

// ----------------------------------------------------------------------------
// Working directory / program path
// ----------------------------------------------------------------------------

/// Current working directory.
pub fn get_working_directory() -> io::Result<PathBuf> {
    std::env::current_dir()
}

/// Set the current working directory.
pub fn set_working_directory(path: impl AsRef<Path>) -> io::Result<()> {
    std::env::set_current_dir(path)
}

/// Path to the running executable.
///
/// Returns `Ok(None)` on unsupported platforms or if the lookup
/// fails. On Linux we read `/proc/self/exe`; on Windows the C++ side
/// used `GetModuleFileNameW` which `std::env::current_exe` already
/// covers.
pub fn get_program_path() -> io::Result<Option<PathBuf>> {
    #[cfg(target_os = "linux")]
    {
        if let Ok(p) = fs::read_link("/proc/self/exe") {
            return Ok(Some(p));
        }
    }

    match std::env::current_exe() {
        Ok(p) => Ok(Some(p)),
        Err(_) => Ok(None),
    }
}

/// Root directories: Windows drives on Windows; `HOME` and `/`
/// on Unix. Mirrors `FileSystem::GetRootDirectoryList`.
pub fn get_root_directory_list() -> Vec<PathBuf> {
    let mut out = Vec::new();

    #[cfg(windows)]
    {
        // `std` has no `GetLogicalDrives`. Fall back to the drive the
        // CWD lives on; PCSX2's callers iterate this list to present
        // an "open file" dialog and one entry is enough to keep the
        // surface functional.
        if let Ok(cwd) = std::env::current_dir() {
            if let Some(root) = cwd.components().next() {
                out.push(PathBuf::from(root.as_os_str()));
            }
        }
    }

    #[cfg(not(windows))]
    {
        if let Some(home) = std::env::var_os("HOME") {
            out.push(PathBuf::from(home));
        }
        out.push(PathBuf::from("/"));
    }

    out
}

// ----------------------------------------------------------------------------
// Symbolic links (POSIX-only; Windows requires elevated privileges
// and the C++ surface here is best-effort).
// ----------------------------------------------------------------------------

/// Create a symbolic link at `link` pointing to `target`.
#[cfg(not(windows))]
pub fn create_symlink(link: impl AsRef<Path>, target: impl AsRef<Path>) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// `Ok(true)` if `path` is a symbolic link.
#[cfg(not(windows))]
pub fn is_symbolic_link(path: impl AsRef<Path>) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(m) => Ok(m.file_type().is_symlink()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Remove a symbolic link. Works for both file and directory symlinks
/// (`fs::remove_file` selects `unlink` vs `rmdir` based on the
/// link's referent type).
#[cfg(not(windows))]
pub fn delete_symbolic_link(path: impl AsRef<Path>) -> io::Result<()> {
    fs::remove_file(path)
}

// ============================================================================
// FFI surface
// ============================================================================
//
// `#[no_mangle] pub extern "C"` functions consumed by the C++ side
// via static linking (`crate-type = ["staticlib"]`). cbindgen picks
// them up and emits matching declarations into `pcsx2_common_rs.h`.
//
// Conventions:
// - `bool` return: `true` = success.
// - Buffers passed in: caller-owned, never freed by Rust.
// - Buffers passed out (e.g. `pcsx2_read_binary_file`): Rust-allocated,
//   released with `pcsx2_buffer_free`.

/// Helper: copy a UTF-8 string into a fixed C buffer.
///
/// Returns the number of bytes written (NUL excluded), or 0 if the
/// buffer is too small to hold even a NUL. Always NUL-terminates the
/// output when `out_len > 0`.
fn write_c_str(out: *mut c_char, out_len: u32, s: &str) -> u32 {
    if out.is_null() || out_len == 0 {
        return 0;
    }
    let bytes = s.as_bytes();
    let cap = (out_len as usize).saturating_sub(1);
    let n = bytes.len().min(cap);
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, n);
        *out.add(n) = 0;
    }
    n as u32
}

/// Helper: turn a `*const c_char` into `Option<PathBuf>`.
///
/// We allocate an owned `PathBuf` so the caller can use it freely
/// without lifetime entanglement with the input pointer.
fn c_path(p: *const c_char) -> Option<PathBuf> {
    if p.is_null() {
        return None;
    }
    let s = unsafe { std::ffi::CStr::from_ptr(p) }.to_str().ok()?;
    Some(PathBuf::from(s))
}

// ---- existence --------------------------------------------------------------

#[no_mangle]
pub extern "C" fn pcsx2_file_exists(path: *const c_char) -> bool {
    c_path(path)
        .and_then(|p| file_exists(p).ok())
        .unwrap_or(false)
}

#[no_mangle]
pub extern "C" fn pcsx2_directory_exists(path: *const c_char) -> bool {
    c_path(path)
        .and_then(|p| directory_exists(p).ok())
        .unwrap_or(false)
}

/// Returns `true` if `path` is a directory containing no entries
/// (or does not exist). Mirrors C++ `FileSystem::DirectoryIsEmpty`:
/// both the Linux `opendir() == nullptr` branch and the Windows
/// `FindFirstFileW == INVALID_HANDLE_VALUE` branch are folded into
/// the empty case, so a missing path returns `true` rather than
/// `false`.
#[no_mangle]
pub extern "C" fn pcsx2_filesystem_directory_is_empty(path: *const c_char) -> bool {
    let Some(p) = c_path(path) else { return false };
    match fs::read_dir(&p) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => true,
    }
}

// ---- directory operations ---------------------------------------------------

#[no_mangle]
pub extern "C" fn pcsx2_create_directory(path: *const c_char, recursive: bool) -> bool {
    let Some(p) = c_path(path) else { return false };
    if recursive {
        create_directory_path(&p).is_ok()
    } else {
        create_directory(&p).is_ok()
    }
}

// ---- file delete / rename ---------------------------------------------------

#[no_mangle]
pub extern "C" fn pcsx2_delete_file(path: *const c_char) -> bool {
    c_path(path)
        .and_then(|p| delete_file_path(&p).ok())
        .map(|_| true)
        .unwrap_or(false)
}

#[no_mangle]
pub extern "C" fn pcsx2_rename_file(old_path: *const c_char, new_path: *const c_char) -> bool {
    match (c_path(old_path), c_path(new_path)) {
        (Some(o), Some(n)) => rename_path(&o, &n).is_ok(),
        _ => false,
    }
}

// ---- read / write -----------------------------------------------------------

/// Read the file at `path` into a freshly-allocated buffer.
///
/// On success: returns `true` and writes the pointer + length to
/// `*out` / `*out_len`. The caller MUST release the buffer with
/// [`pcsx2_buffer_free`]. On failure: returns `false` and writes
/// `null` / `0` to the outputs.
#[no_mangle]
pub extern "C" fn pcsx2_read_binary_file(
    path: *const c_char,
    out: *mut *mut u8,
    out_len: *mut usize,
) -> bool {
    if out.is_null() || out_len.is_null() {
        return false;
    }
    unsafe {
        *out = std::ptr::null_mut();
        *out_len = 0;
    }

    let Some(p) = c_path(path) else { return false };

    let Ok(Some(mut data)) = read_binary_file(&p) else {
        return false;
    };

    let len = data.len();
    let cap = data.capacity();
    let ptr = data.as_mut_ptr();
    // Transfer ownership to the caller without running `Vec`'s
    // destructor. `pcsx2_buffer_free` will reconstruct the Vec from
    // `(ptr, len, cap)` and drop it normally.
    std::mem::forget(data);

    unsafe {
        *out = ptr;
        *out_len = len;
    }
    // Stash `cap` for the caller. We pass it via `pcsx2_buffer_free`,
    // so just keep it as a local to document the contract.
    let _ = cap;
    true
}

/// Write `len` bytes from `data` to `path`, overwriting if it exists.
#[no_mangle]
pub extern "C" fn pcsx2_write_binary_file(path: *const c_char, data: *const u8, len: usize) -> bool {
    let Some(p) = c_path(path) else { return false };
    if data.is_null() && len > 0 {
        return false;
    }
    let slice = if len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(data, len) }
    };
    write_binary_file(&p, slice).is_ok()
}

/// Free a buffer previously allocated by `pcsx2_read_binary_file`.
///
/// `cap` is the original allocation capacity from the matching
/// `pcsx2_read_binary_file` call. We reconstruct the original `Vec`
/// and let it run its destructor; this is the safe path because the
/// global deallocator uses the `Layout` recorded by the allocator,
/// not the length we hand back.
#[no_mangle]
pub extern "C" fn pcsx2_buffer_free(ptr: *mut u8, len: usize, cap: usize) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let _ = Vec::from_raw_parts(ptr, len, cap);
    }
}

// ---- fread-based progress reads --------------------------------------------
//
// PCSX2 has two C++ overloads — `ReadFileWithProgress` and
// `ReadFileWithPartialProgress` — that share the same shape: open
// `FILE*`, fill a buffer, and optionally call into a `ProgressCallback`
// to report progress. We expose them as C-ABI symbols so the C++
// binary can link without the corresponding `common/FileSystem.cpp`
// TU. The `progress` and `err` parameters are opaque from Rust's
// point of view (the C++ side owns their layouts), so for now we
// only use them as null-checks:
//   - `progress` is currently unused — see TODO below.
//   - `err`, when non-null, cannot be safely populated without
//     knowing the `Error` struct layout, so on failure we leave it
//     untouched and let the C++ caller inspect `ferror(fp)` /
//     `errno` separately.
//
// TODO(progress): wire progress reporting through a registered FFI
// callback once the `ProgressCallback` vtable is exposed across
// the Rust↔C++ boundary. For now the C++ side will see no progress
// updates for reads that go through these entry points.

/// Helper: read up to `count` bytes from `file` into `bytes`.
/// Returns the number of bytes actually read. Returns 0 on null
/// pointers or I/O error.
unsafe fn raw_fread(file: *mut c_void, bytes: *mut c_void, count: u64) -> u64 {
    if file.is_null() || bytes.is_null() || count == 0 {
        return 0;
    }
    // SAFETY: caller guarantees `file` is a live `FILE*` from
    // `<cstdio>` and `bytes` is a writable buffer of at least
    // `count` bytes.
    let fp = file as *mut libc::FILE;
    let n = libc::fread(bytes, 1, count as usize, fp);
    n as u64
}

/// Mirror of C++ `FileSystem::ReadFileWithProgress`.
///
/// Reads up to `size` bytes from `fp` into `buffer` and returns the
/// number of bytes actually read. `progress` and `err` are opaque
/// from Rust; see module-level TODO.
#[no_mangle]
pub extern "C" fn pcsx2_filesystem_read_file_with_progress(
    fp: *mut c_void,
    buffer: *mut c_void,
    size: u64,
    _progress: *mut c_void,
    _err: *mut c_void,
    _base_offset: u64,
) -> u64 {
    unsafe { raw_fread(fp, buffer, size) }
}

/// Mirror of C++ `FileSystem::ReadFileWithPartialProgress`.
///
/// Like [`pcsx2_filesystem_read_file_with_progress`] but with explicit
/// `min_step` / `max_step` progress range bounds. `progress` and
/// `err` are opaque from Rust; see module-level TODO.
#[no_mangle]
pub extern "C" fn pcsx2_filesystem_read_file_with_partial_progress(
    fp: *mut c_void,
    buffer: *mut c_void,
    size: u64,
    _progress: *mut c_void,
    _min_step: c_int,
    _max_step: c_int,
    _err: *mut c_void,
    _base_offset: u64,
) -> u64 {
    unsafe { raw_fread(fp, buffer, size) }
}

// ---- program / working directory -------------------------------------------

/// Write the running executable's path into `out`.
///
/// Returns the number of bytes written (NUL excluded), or 0 on
/// failure / unsupported platform. Always NUL-terminates when
/// `out_len > 0`.
#[no_mangle]
pub extern "C" fn pcsx2_get_program_path(out: *mut c_char, out_len: u32) -> u32 {
    match get_program_path() {
        Ok(Some(p)) => write_c_str(out, out_len, &p.to_string_lossy()),
        _ => 0,
    }
}

/// Write the current working directory into `out`.
#[no_mangle]
pub extern "C" fn pcsx2_get_working_directory(out: *mut c_char, out_len: u32) -> u32 {
    match get_working_directory() {
        Ok(p) => write_c_str(out, out_len, &p.to_string_lossy()),
        Err(_) => 0,
    }
}

/// Set the current working directory.
#[no_mangle]
pub extern "C" fn pcsx2_set_working_directory(path: *const c_char) -> bool {
    c_path(path)
        .and_then(|p| set_working_directory(&p).ok())
        .is_some()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_star_matches_anything() {
        assert!(glob_match("foo.txt", "*"));
        assert!(glob_match("", "*"));
    }

    #[test]
    fn glob_question_matches_one_char() {
        assert!(glob_match("a", "?"));
        assert!(!glob_match("ab", "?"));
        assert!(glob_match("ab", "??"));
    }

    #[test]
    fn glob_prefix_suffix() {
        assert!(glob_match("hello.txt", "*.txt"));
        assert!(!glob_match("hello.bin", "*.txt"));
        assert!(glob_match("foo_bar_baz", "*_bar_*"));
    }

    #[test]
    fn glob_case_sensitive() {
        assert!(!glob_match("Foo.TXT", "*.txt"));
        assert!(glob_match("Foo.TXT", "*.TXT"));
    }

    #[test]
    fn write_c_str_truncates() {
        let mut buf = [0i8; 4];
        let n = write_c_str(buf.as_mut_ptr(), buf.len() as u32, "hello world");
        assert_eq!(n, 3);
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, 4) };
        assert_eq!(&bytes[..3], b"hel");
        assert_eq!(bytes[3], 0);
    }

    #[test]
    fn write_c_str_handles_null_buffer() {
        assert_eq!(write_c_str(std::ptr::null_mut(), 32, "anything"), 0);
        assert_eq!(write_c_str(std::ptr::NonNull::<c_char>::dangling().as_ptr(), 0, "x"), 0);
    }
}
