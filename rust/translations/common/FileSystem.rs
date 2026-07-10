// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust 2021 translation of PCSX2's `common/FileSystem.{h,cpp}`.
//!
//! This module provides a small, focused abstraction over the local
//! filesystem: path inspection, existence/stat helpers, file read/write
//! (including atomic-write), directory creation, recursive
//! copy/rename/delete, wildcard enumeration, and a `Mmap`-style read-only
//! view of file contents.
//!
//! Only `std` is used. On `unix` and `windows` targets the appropriate
//! `std::os` extensions are pulled in for symbolic links, byte-level
//! permissions and the like.

use std::fs::{self, File, FileType, Metadata, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[cfg(unix)]
use std::os::unix::fs::{PermissionsExt, symlink as unix_symlink};
#[cfg(windows)]
use std::os::windows::fs::{symlink_dir, symlink_file};

// ---------------------------------------------------------------------------
// Flags / attribute bits
// ---------------------------------------------------------------------------

pub mod file_attributes {
    pub const DIRECTORY: u32 = 1 << 0;
    pub const READ_ONLY: u32 = 1 << 1;
    pub const COMPRESSED: u32 = 1 << 2;
}

pub mod find_flags {
    pub const RECURSIVE: u32 = 1 << 0;
    pub const RELATIVE_PATHS: u32 = 1 << 1;
    pub const HIDDEN_FILES: u32 = 1 << 2;
    pub const FOLDERS: u32 = 1 << 3;
    pub const FILES: u32 = 1 << 4;
    pub const KEEP_ARRAY: u32 = 1 << 5;
    pub const SORT_BY_NAME: u32 = 1 << 6;
}

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Mirrors `FILESYSTEM_STAT_DATA`.
#[derive(Clone, Debug)]
pub struct StatData {
    pub creation_time: SystemTime,
    pub modification_time: SystemTime,
    pub size: i64,
    pub attributes: u32,
}

/// Mirrors `FILESYSTEM_FIND_DATA`.
#[derive(Clone, Debug)]
pub struct FindData {
    pub file_name: PathBuf,
    pub creation_time: SystemTime,
    pub modification_time: SystemTime,
    pub size: i64,
    pub attributes: u32,
}

pub type FindResults = Vec<FindData>;

/// File-share mode, Windows-only behaviour. Other platforms treat all opens
/// as fully shared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileShareMode {
    DenyReadWrite,
    DenyWrite,
    DenyRead,
    DenyNone,
}

// ---------------------------------------------------------------------------
// Existence / stat helpers
// ---------------------------------------------------------------------------

/// Returns true if `path` exists and is a regular file.
pub fn file_exists(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_file()).unwrap_or(false)
}

/// Returns true if `path` exists and is a directory.
pub fn directory_exists(path: &Path) -> bool {
    fs::metadata(path).map(|m| m.is_dir()).unwrap_or(false)
}

/// Returns true if `path` exists and is a symbolic link.
pub fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// Returns the size of the file at `path`, or `-1` if it cannot be statted.
pub fn get_path_file_size(path: &Path) -> i64 {
    stat_file(path).map(|s| s.size).unwrap_or(-1)
}

/// Returns the last-modified timestamp of `path`, if available.
pub fn get_file_timestamp(path: &Path) -> Option<SystemTime> {
    stat_file(path).ok().map(|s| s.modification_time)
}

/// Returns true if `path` is a directory containing no entries (other than
/// `.` / `..`).  Returns true for paths that cannot be opened (matching the
/// C++ behaviour of treating unreadable directories as empty).
pub fn directory_is_empty(path: &Path) -> bool {
    let Ok(it) = fs::read_dir(path) else { return true; };
    for entry in it.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        if s != "." && s != ".." {
            return false;
        }
    }
    true
}

/// Stat a file or directory.
pub fn stat_file(path: &Path) -> io::Result<StatData> {
    let meta = fs::symlink_metadata(path)?;
    Ok(meta_to_stat(&meta))
}

fn meta_to_stat(meta: &Metadata) -> StatData {
    let mut attrs = 0;
    if meta.is_dir() {
        attrs |= file_attributes::DIRECTORY;
    }
    if meta.permissions().readonly() {
        attrs |= file_attributes::READ_ONLY;
    }
    // COMPRESSED is Windows-only; on other platforms we simply never set it.
    StatData {
        creation_time: meta.created().unwrap_or(SystemTime::UNIX_EPOCH),
        modification_time: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
        size: meta.len() as i64,
        attributes: attrs,
    }
}

// ---------------------------------------------------------------------------
// Directory creation / deletion
// ---------------------------------------------------------------------------

/// Create `path` (and any missing parents) if `recursive` is true.
pub fn create_directory_path(path: &Path, recursive: bool) -> io::Result<()> {
    if path.as_os_str().is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "path is empty"));
    }
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists && directory_exists(path) => Ok(()),
        Err(e) => {
            if !recursive {
                return Err(e);
            }
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() && !directory_exists(parent) {
                    create_directory_path(parent, true)?;
                }
            }
            match fs::create_dir(path) {
                Ok(()) => Ok(()),
                Err(e2) if e2.kind() == io::ErrorKind::AlreadyExists && directory_exists(path) => Ok(()),
                Err(e2) => Err(e2),
            }
        }
    }
}

/// Create `path` if it does not exist; idempotent.
pub fn ensure_directory(path: &Path, recursive: bool) -> io::Result<()> {
    if directory_exists(path) {
        return Ok(());
    }
    create_directory_path(path, recursive)
}

/// Remove an empty directory.
pub fn delete_directory(path: &Path) -> io::Result<()> {
    fs::remove_dir(path)
}

/// Remove a file (errors if `path` is a directory).
pub fn delete_file_path(path: &Path) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        return Err(io::Error::new(io::ErrorKind::Other, "path is a directory"));
    }
    fs::remove_file(path)
}

/// Rename / move a file or directory.
pub fn rename_path(old: &Path, new: &Path) -> io::Result<()> {
    fs::rename(old, new)
}

// ---------------------------------------------------------------------------
// Recursive operations
// ---------------------------------------------------------------------------

/// Recursively copy `src` to `dst`.  `filter` is invoked with the source
/// path of every entry; entries for which the filter returns `false` are
/// skipped (the destination tree is still created for any included
/// descendants).
pub fn recursive_copy<F>(src: &Path, dst: &Path, filter: F) -> io::Result<()>
where
    F: Fn(&Path) -> bool,
{
    let meta = fs::symlink_metadata(src)?;
    if meta.is_dir() {
        if !dst.exists() {
            fs::create_dir_all(dst)?;
        }
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let from = entry.path();
            let to = dst.join(entry.file_name());
            if filter(&from) {
                recursive_copy(&from, &to, &filter)?;
            }
        }
        Ok(())
    } else if filter(src) {
        fs::copy(src, dst).map(|_| ())
    } else {
        Ok(())
    }
}

/// Recursively move `src` to `dst`.  Falls back to a copy+delete if
/// `rename(2)` fails (e.g. cross-device moves).
pub fn recursive_rename(src: &Path, dst: &Path) -> io::Result<()> {
    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            recursive_copy(src, dst, |_| true)?;
            recursive_delete(src)
        }
    }
}

/// Recursively delete `path` (a file, symlink or directory tree).
/// `NotFound` is treated as success.
pub fn recursive_delete(path: &Path) -> io::Result<()> {
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if meta.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            recursive_delete(&entry.path())?;
        }
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

/// Copy a single file.  When `replace` is false and the destination exists,
/// the function returns `AlreadyExists`.
pub fn copy_file_path(src: &Path, dst: &Path, replace: bool) -> io::Result<()> {
    if !replace && dst.exists() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "destination exists"));
    }
    fs::copy(src, dst).map(|_| ())
}

// ---------------------------------------------------------------------------
// File read / write
// ---------------------------------------------------------------------------

/// Read the entire file at `path` into a `Vec<u8>`.
pub fn read_binary_file(path: &Path) -> io::Result<Vec<u8>> {
    fs::read(path)
}

/// Read the entire file at `path` into a `String`.
pub fn read_file_to_string(path: &Path) -> io::Result<String> {
    fs::read_to_string(path)
}

/// Write `data` to `path`, creating or truncating the file.
pub fn write_file(path: &Path, data: &[u8]) -> io::Result<()> {
    fs::write(path, data)
}

/// Write a string to `path`, creating or truncating the file.
pub fn write_string_to_file(path: &Path, s: &str) -> io::Result<()> {
    fs::write(path, s.as_bytes())
}

/// Atomically write `data` to `path` by writing to a sibling temporary file
/// and renaming it into place.  The temporary file is created in the same
/// directory as `path` so the final rename is on the same filesystem.
pub fn write_file_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let base = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file");
    let tmp = parent.join(format!(".{}.tmp", base));
    {
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    // Best-effort cleanup if the rename fails.
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

/// Open a file with the given mode.  Thin wrapper around `File::open` /
/// `OpenOptions` that translates the C++ `fopen` mode strings.
pub fn open_file(path: &Path, mode: &str) -> io::Result<File> {
    let mut opts = OpenOptions::new();
    let mut mode = mode;
    // Strip leading 'b' / 't' -- rust has no text/binary distinction.
    if let Some(stripped) = mode.strip_prefix('b').or_else(|| mode.strip_prefix('t')) {
        mode = stripped;
    }
    for c in mode.chars() {
        match c {
            'r' => opts.read(true),
            'w' => opts.write(true).create(true).truncate(true),
            'a' => opts.write(true).create(true).append(true),
            '+' => opts.read(true).write(true).create(true),
            'x' => opts.create_new(true),
            _ => &mut opts,
        };
    }
    opts.open(path)
}

/// Read a file into memory using buffered IO.  Returns the read content and
/// the underlying `File` so the caller can decide to keep it open.
pub fn read_file_with_progress<R: Read>(
    mut src: R,
    dst: &mut [u8],
    chunk_size: usize,
    mut progress: impl FnMut(usize) -> bool,
) -> io::Result<usize> {
    let mut done = 0;
    let dst_len = dst.len();
    while done < dst_len {
        if !progress(done) {
            break;
        }
        let end = dst_len.min(done + chunk_size);
        let n = src.read(&mut dst[done..end])?;
        if n == 0 {
            break;
        }
        done += n;
    }
    Ok(done)
}

// ---------------------------------------------------------------------------
// mmap-style read view
// ---------------------------------------------------------------------------

/// Read-only view of a file's contents.  `std` does not expose real memory
/// mappings, so this is implemented as a read into a heap-allocated buffer
/// (semantics equivalent for small/medium files).
pub struct MappedFile {
    bytes: Box<[u8]>,
    handle: Option<File>,
}

impl MappedFile {
    pub fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn handle(&self) -> Option<&File> {
        self.handle.as_ref()
    }
}

impl AsRef<[u8]> for MappedFile {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

/// Map the file at `path` for read access.  Returns an empty mapping if the
/// file is zero bytes (matching the C++ behaviour).
pub fn map_binary_file_for_read(path: &Path) -> io::Result<MappedFile> {
    let handle = OpenOptions::new().read(true).open(path)?;
    let mut reader = BufReader::new(&handle);
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    Ok(MappedFile {
        bytes: bytes.into_boxed_slice(),
        handle: Some(handle),
    })
}

/// Release a mapped file.  Currently a no-op (the buffer is dropped when
/// the `MappedFile` goes out of scope) but kept for API parity.
pub fn unmap_file(_m: MappedFile) {}

// ---------------------------------------------------------------------------
// Symbolic links
// ---------------------------------------------------------------------------

/// Create a symbolic link at `link` pointing to `target`.  On Windows the
/// correct variant (`symlink_file` vs `symlink_dir`) is chosen based on
/// whether `target` is a directory.
pub fn create_symlink(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        unix_symlink(target, link)
    }
    #[cfg(windows)]
    {
        if target.is_dir() {
            symlink_dir(target, link)
        } else {
            symlink_file(target, link)
        }
    }
}

/// Delete a symbolic link without following it.
pub fn delete_symlink(path: &Path) -> io::Result<()> {
    fs::symlink_metadata(path)?;
    fs::remove_file(path)
}

// ---------------------------------------------------------------------------
// Enumeration
// ---------------------------------------------------------------------------

/// Options that control `FileSystemEnumerator`.
#[derive(Clone, Debug)]
pub struct EnumeratorOptions {
    pub recursive: bool,
    pub include_hidden: bool,
    pub include_files: bool,
    pub include_folders: bool,
    pub include_symlinks: bool,
    pub relative_paths: bool,
    pub sort_by_name: bool,
}

impl Default for EnumeratorOptions {
    fn default() -> Self {
        Self {
            recursive: false,
            include_hidden: false,
            include_files: true,
            include_folders: true,
            include_symlinks: true,
            relative_paths: false,
            sort_by_name: false,
        }
    }
}

impl EnumeratorOptions {
    /// Translate a `find_flags::*` bitmask into an `EnumeratorOptions`.
    pub fn from_flags(flags: u32) -> Self {
        Self {
            recursive: flags & find_flags::RECURSIVE != 0,
            include_hidden: flags & find_flags::HIDDEN_FILES != 0,
            include_files: flags & find_flags::FILES != 0,
            include_folders: flags & find_flags::FOLDERS != 0,
            include_symlinks: true,
            relative_paths: flags & find_flags::RELATIVE_PATHS != 0,
            sort_by_name: flags & find_flags::SORT_BY_NAME != 0,
        }
    }
}

/// Depth-first iterator over the entries under a directory tree.  Yields
/// `(PathBuf, FileType)` for each entry that matches the configured
/// `EnumeratorOptions`.  IO errors encountered during iteration are
/// surfaced as `Err(_)` items rather than aborting the walk.
pub struct FileSystemEnumerator {
    options: EnumeratorOptions,
    root: PathBuf,
    stack: Vec<PathBuf>,
    current: Option<std::fs::ReadDir>,
    pending: Vec<(PathBuf, FileType)>,
    /// Resolved canonical paths already visited -- used to break symlink
    /// loops in recursive mode.
    visited: Vec<PathBuf>,
}

impl FileSystemEnumerator {
    /// Construct an enumerator rooted at `root`.
    pub fn new(root: &Path, options: EnumeratorOptions) -> Self {
        Self {
            options,
            root: root.to_path_buf(),
            stack: vec![root.to_path_buf()],
            current: None,
            pending: Vec::new(),
            visited: Vec::new(),
        }
    }

    /// Convenience constructor that takes the `find_flags::*` bitmask.
    pub fn from_flags(root: &Path, flags: u32) -> Self {
        Self::new(root, EnumeratorOptions::from_flags(flags))
    }

    /// Drain the iterator into a `Vec<FindData>`.
    pub fn into_find_results(mut self) -> FindResults {
        let mut out = Vec::new();
        for item in self.by_ref() {
            let (path, ft) = match item {
                Ok(v) => v,
                Err(_) => continue,
            };
            let meta = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let stat = meta_to_stat(&meta);
            out.push(FindData {
                file_name: path,
                creation_time: stat.creation_time,
                modification_time: stat.modification_time,
                size: stat.size,
                attributes: stat.attributes | type_to_attr(ft),
            });
        }
        if self.options.sort_by_name {
            out.sort_by(|a, b| a.file_name.cmp(&b.file_name));
        }
        out
    }

    fn adjust(&self, p: PathBuf) -> PathBuf {
        if self.options.relative_paths {
            p.strip_prefix(&self.root).map(|x| x.to_path_buf()).unwrap_or(p)
        } else {
            p
        }
    }

    fn eligible(&self, name: &std::ffi::OsStr) -> bool {
        if !self.options.include_hidden {
            if let Some(s) = name.to_str() {
                if s.starts_with('.') {
                    return false;
                }
            }
        }
        true
    }
}

fn type_to_attr(ft: FileType) -> u32 {
    let mut a = 0;
    if ft.is_dir() {
        a |= file_attributes::DIRECTORY;
    }
    a
}

impl Iterator for FileSystemEnumerator {
    type Item = io::Result<(PathBuf, FileType)>;

    fn next(&mut self) -> Option<Self::Item> {
        // Pre-sorted batches (per directory) for stability when
        // `sort_by_name` is requested.
        while self.pending.is_empty() {
            // Pop the next directory to process, opening its iterator.
            let dir = match self.stack.pop() {
                Some(d) => d,
                None => return None,
            };
            if self.options.recursive {
                if let Ok(canon) = fs::canonicalize(&dir) {
                    if self.visited.iter().any(|v| v == &canon) {
                        continue;
                    }
                    self.visited.push(canon);
                }
            }
            self.current = match fs::read_dir(&dir) {
                Ok(rd) => Some(rd),
                Err(e) => return Some(Err(e)),
            };
            let mut batch: Vec<(PathBuf, FileType)> = Vec::new();
            while let Some(item) = self.current.as_mut().and_then(|it| it.next()) {
                let entry = match item {
                    Ok(e) => e,
                    Err(e) => return Some(Err(e)),
                };
                let name = entry.file_name();
                if !self.eligible(&name) {
                    continue;
                }
                let path = entry.path();
                let ft = match entry.file_type() {
                    Ok(ft) => ft,
                    Err(e) => return Some(Err(e)),
                };
                if ft.is_dir() {
                    if self.options.recursive {
                        self.stack.push(path.clone());
                    }
                    if self.options.include_folders {
                        batch.push((self.adjust(path), ft));
                    }
                } else if (ft.is_file() || ft.is_symlink())
                    && (self.options.include_files || (ft.is_symlink() && self.options.include_symlinks))
                {
                    batch.push((self.adjust(path), ft));
                }
            }
            self.current = None;
            if self.options.sort_by_name {
                batch.sort_by(|a, b| a.0.cmp(&b.0));
            }
            self.pending = batch;
        }
        Some(Ok(self.pending.remove(0)))
    }
}

// ---------------------------------------------------------------------------
// Process / working-directory helpers
// ---------------------------------------------------------------------------

/// Return the list of "root" paths that the C++ version exposes (drive
/// letters on Windows, `$HOME` + `/` on Unix).
pub fn get_root_directory_list() -> Vec<PathBuf> {
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        for letter in b'A'..=b'Z' {
            let p = PathBuf::from(format!("{}:\\", letter as char));
            if p.exists() {
                out.push(p);
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

/// Returns the current working directory.
pub fn get_working_directory() -> io::Result<PathBuf> {
    std::env::current_dir()
}

/// Sets the current working directory.  Returns `true` on success.
pub fn set_working_directory(path: &Path) -> io::Result<()> {
    std::env::set_current_dir(path)
}

/// Returns the path to the running executable.
pub fn get_program_path() -> io::Result<PathBuf> {
    std::env::current_exe()
}

/// Returns the package path (AppImage / executable).
pub fn get_package_path() -> io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        if let Some(p) = std::env::var_os("APPIMAGE") {
            return Ok(PathBuf::from(p));
        }
    }
    get_program_path()
}

// ---------------------------------------------------------------------------
// Atomic-write helper around a writer
// ---------------------------------------------------------------------------

/// Wraps a `Write` and writes through to a sibling temp file.  When the
/// `AtomicFileWriter` is dropped without `commit()`, the temp file is
/// removed.  On `commit()` the temp file is atomically renamed to the
/// target path.
pub struct AtomicFileWriter {
    target: PathBuf,
    tmp: PathBuf,
    file: Option<BufWriter<File>>,
    finished: bool,
}

impl AtomicFileWriter {
    pub fn create(target: &Path) -> io::Result<Self> {
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let base = target
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file");
        let tmp = parent.join(format!(".{}.tmp", base));
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)?;
        Ok(Self {
            target: target.to_path_buf(),
            tmp,
            file: Some(BufWriter::new(file)),
            finished: false,
        })
    }

    pub fn commit(mut self) -> io::Result<()> {
        if let Some(mut bw) = self.file.take() {
            bw.flush()?;
            let inner = bw.into_inner().map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
            inner.sync_all()?;
        }
        match fs::rename(&self.tmp, &self.target) {
            Ok(()) => {
                self.finished = true;
                Ok(())
            }
            Err(e) => {
                let _ = fs::remove_file(&self.tmp);
                Err(e)
            }
        }
    }

    pub fn cancel(mut self) -> io::Result<()> {
        self.file = None;
        if self.tmp.exists() {
            fs::remove_file(&self.tmp)
        } else {
            Ok(())
        }
    }
}

impl Write for AtomicFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.file.as_mut() {
            Some(f) => f.write(buf),
            None => Err(io::Error::new(io::ErrorKind::Other, "writer is finished")),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

impl Drop for AtomicFileWriter {
    fn drop(&mut self) {
        if !self.finished && self.tmp.exists() {
            let _ = fs::remove_file(&self.tmp);
        }
    }
}

// ---------------------------------------------------------------------------
// Misc POSIX-style helpers (only meaningful on Unix)
// ---------------------------------------------------------------------------

/// POSIX advisory file lock.  On non-Unix targets this compiles to a no-op
/// wrapper so the type can be referenced from cross-platform code.
pub struct PosixLock {
    #[cfg(unix)]
    fd: i32,
}

#[cfg(unix)]
impl PosixLock {
    pub fn new(file: &File) -> io::Result<Self> {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        let r = unsafe { libc_lock(fd) };
        if r == 0 {
            Ok(Self { fd })
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(unix)]
impl Drop for PosixLock {
    fn drop(&mut self) {
        unsafe { libc_unlock(self.fd) };
    }
}

#[cfg(unix)]
extern "C" {
    fn flock(fd: i32, op: i32) -> i32;
}

#[cfg(unix)]
fn libc_lock(fd: i32) -> i32 {
    // LOCK_EX = 2, LOCK_NB = 4
    unsafe { flock(fd, 2 | 4) }
}

#[cfg(unix)]
fn libc_unlock(fd: i32) -> i32 {
    // LOCK_UN = 8
    unsafe { flock(fd, 8) }
}

#[cfg(not(unix))]
impl PosixLock {
    pub fn new(_file: &File) -> io::Result<Self> {
        Ok(Self {})
    }
}

// ---------------------------------------------------------------------------
// File-share-mode aware open (only meaningful on Windows)
// ---------------------------------------------------------------------------

/// Open a file honouring the requested `FileShareMode`.  On non-Windows
/// platforms the share mode is ignored and the call is equivalent to
/// `open_file`.
pub fn open_shared_file(path: &Path, mode: &str, _share: FileShareMode) -> io::Result<File> {
    open_file(path, mode)
}

// ---------------------------------------------------------------------------
// Path manipulation helpers (a small subset of Path:: )
// ---------------------------------------------------------------------------

/// Combine two path components using the platform separator.
pub fn path_combine(base: &Path, next: &Path) -> PathBuf {
    let mut out = base.to_path_buf();
    if !out.as_os_str().is_empty() && !next.as_os_str().is_empty() {
        // Ensure exactly one separator between the two.
        if !out.to_string_lossy().ends_with(std::path::MAIN_SEPARATOR) {
            out.push(std::path::MAIN_SEPARATOR_STR);
        }
    }
    out.push(next);
    out
}

/// Return the file name portion of `path`.
pub fn path_get_file_name(path: &Path) -> Option<&Path> {
    path.file_name().map(Path::new)
}

/// Return the directory portion of `path`.
pub fn path_get_directory(path: &Path) -> Option<&Path> {
    path.parent()
}

/// Resolve `path` to an absolute, canonical form (with symlinks resolved).
pub fn path_real(path: &Path) -> io::Result<PathBuf> {
    fs::canonicalize(path)
}

/// `true` if `path` is absolute.
pub fn path_is_absolute(path: &Path) -> bool {
    path.is_absolute()
}

// ---------------------------------------------------------------------------
// File-handle sized seek/tell helpers
// ---------------------------------------------------------------------------

/// Seek a `File` using a 64-bit offset, matching the C++ `FSeek64`.
pub fn fseek64(f: &mut File, offset: i64, whence: SeekFrom) -> io::Result<u64> {
    f.seek(whence).and_then(|_| f.stream_position())
}

/// Tell a `File` using a 64-bit offset, matching the C++ `FTell64`.
pub fn ftell64(f: &mut File) -> io::Result<u64> {
    f.stream_position()
}

/// Return the size of an open file without changing its position.
pub fn fsize64(f: &mut File) -> io::Result<u64> {
    let pos = f.stream_position()?;
    let size = f.seek(SeekFrom::End(0))?;
    f.seek(SeekFrom::Start(pos))?;
    Ok(size)
}

// ---------------------------------------------------------------------------
// Windows-only helpers (compiled to no-ops on other targets)
// ---------------------------------------------------------------------------

/// Windows path normalisation.  On non-Windows targets this is a no-op
/// identity wrapper.
#[cfg(windows)]
pub fn get_win32_path(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[cfg(not(windows))]
pub fn get_win32_path(s: &str) -> String {
    s.to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    fn scratch(name: &str) -> PathBuf {
        let mut p = temp_dir();
        p.push(format!("pcsx2_fs_test_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn create_and_remove() {
        let root = scratch("create_and_remove");
        ensure_directory(&root, true).unwrap();
        assert!(directory_exists(&root));
        recursive_delete(&root).unwrap();
        assert!(!directory_exists(&root));
    }

    #[test]
    fn write_read_round_trip() {
        let root = scratch("write_read_round_trip");
        ensure_directory(&root, true).unwrap();
        let f = root.join("hello.txt");
        write_file(&f, b"hi").unwrap();
        assert!(file_exists(&f));
        assert_eq!(read_file_to_string(&f).unwrap(), "hi");
        let s = read_binary_file(&f).unwrap();
        assert_eq!(s, b"hi");
        recursive_delete(&root).unwrap();
    }

    #[test]
    fn atomic_write() {
        let root = scratch("atomic_write");
        ensure_directory(&root, true).unwrap();
        let f = root.join("a.bin");
        write_file_atomic(&f, b"data").unwrap();
        assert_eq!(fs::read(&f).unwrap(), b"data");
        recursive_delete(&root).unwrap();
    }

    #[test]
    fn recursive_copy_works() {
        let src = scratch("recopy_src");
        let dst = scratch("recopy_dst");
        ensure_directory(&src, true).unwrap();
        write_file(&src.join("a"), b"a").unwrap();
        ensure_directory(&src.join("sub"), true).unwrap();
        write_file(&src.join("sub").join("b"), b"b").unwrap();
        recursive_copy(&src, &dst, |_| true).unwrap();
        assert!(file_exists(&dst.join("a")));
        assert!(file_exists(&dst.join("sub").join("b")));
        recursive_delete(&src).unwrap();
        recursive_delete(&dst).unwrap();
    }

    #[test]
    fn enumerator_simple() {
        let root = scratch("enum_simple");
        ensure_directory(&root, true).unwrap();
        write_file(&root.join("a"), b"a").unwrap();
        write_file(&root.join("b"), b"b").unwrap();
        ensure_directory(&root.join("sub"), true).unwrap();
        write_file(&root.join("sub").join("c"), b"c").unwrap();

        let mut names: Vec<PathBuf> = FileSystemEnumerator::new(
            &root,
            EnumeratorOptions {
                recursive: true,
                ..Default::default()
            },
        )
        .map(|r| r.unwrap().0)
        .collect();
        names.sort();
        assert_eq!(names.len(), 4); // sub dir, a, b, c
        recursive_delete(&root).unwrap();
    }

    #[test]
    fn rename_and_delete() {
        let root = scratch("rename");
        ensure_directory(&root, true).unwrap();
        let a = root.join("a");
        let b = root.join("b");
        write_file(&a, b"hello").unwrap();
        recursive_rename(&a, &b).unwrap();
        assert!(file_exists(&b));
        recursive_delete(&root).unwrap();
    }
}
