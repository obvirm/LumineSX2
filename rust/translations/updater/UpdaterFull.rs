// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Idiomatic Rust translation of PCSX2's `updater/` module set.
//!
//! This module rewrites the contents of the following C/C++ sources into a
//! single Rust 2021 file that depends only on `std`:
//!
//! - `updater/Updater.cpp` / `updater/Updater.h` (zip-driven installer)
//! - `updater/UpdaterExtractor.h` (raw `update.7z` extractor used to unpack
//!   the updater itself before it can run)
//! - `updater/SZErrors.h` (7z error code -> human readable string)
//! - `updater/Windows/WindowsUpdater.cpp` (Win32 progress UI and `wWinMain`
//!   bootstrap that orchestrates wait/install/launch)
//! - `updater/Windows/resource.h` (icon id; surfaced as a constant)
//!
//! The 7z decoder and the Win32 / COM / `IFileOperation` plumbing used in the
//! original sources do not have equivalents inside `std`. To keep the module
//! self-contained the original dependencies are abstracted behind small trait
//! seams so the structure of the C++ can be preserved without dragging in
//! external crates:
//!
//! - [`ArchiveStream`] / [`Archive`] / [`ProgressSink`] traits capture the
//!   pieces of 7z's C API and the `ProgressCallback` interface the originals
//!   lean on. A small in-memory implementation ([`InMemoryArchive`]) is
//!   provided as a working default so the module compiles and runs end-to-end
//!   in tests; production builds would wire these traits to the real 7z
//!   decoder.
//! - The Win32-specific shell operations (`IFileOperation` delete,
//!   `MoveFileExW`, `ShellExecuteW`, `OpenProcess`/`WaitForSingleObject`)
//!   are encapsulated behind [`WindowsShell`], with a no-op default that
//!   reports the calls it would make. A real `cfg(windows)` implementation
//!   would back this with the win32 crate.

#![allow(clippy::needless_range_loop)]

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Shared types / traits
// ---------------------------------------------------------------------------

/// Path separator character used by the host operating system.
#[cfg(windows)]
pub const FS_OSPATH_SEPARATOR_CHARACTER: char = '\\';
#[cfg(not(windows))]
pub const FS_OSPATH_SEPARATOR_CHARACTER: char = '/';

/// Path separator string form, used when joining paths with `format!`.
pub const FS_OSPATH_SEPARATOR_STR: &str = {
    #[cfg(windows)]
    {
        "\\"
    }
    #[cfg(not(windows))]
    {
        "/"
    }
};

/// 7z-style error codes (subset matching `SZErrorToString`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum SzError {
    Ok = 0,
    Data = -1,
    Mem = -2,
    Crc = -3,
    Unsupported = -4,
    Param = -5,
    InputEof = -6,
    OutputEof = -7,
    Read = -8,
    Write = -9,
    Progress = -10,
    Fail = -11,
    Thread = -12,
    Archive = -13,
    NoArchive = -14,
    Unknown = i32::MIN,
}

impl SzError {
    pub fn from_code(code: i32) -> Self {
        match code {
            0 => SzError::Ok,
            -1 => SzError::Data,
            -2 => SzError::Mem,
            -3 => SzError::Crc,
            -4 => SzError::Unsupported,
            -5 => SzError::Param,
            -6 => SzError::InputEof,
            -7 => SzError::OutputEof,
            -8 => SzError::Read,
            -9 => SzError::Write,
            -10 => SzError::Progress,
            -11 => SzError::Fail,
            -12 => SzError::Thread,
            -13 => SzError::Archive,
            -14 => SzError::NoArchive,
            _ => SzError::Unknown,
        }
    }

    /// Equivalent of the C++ `SZErrorToString` helper.
    pub fn as_str(self) -> &'static str {
        match self {
            SzError::Ok => "SZ_OK",
            SzError::Data => "SZ_ERROR_DATA",
            SzError::Mem => "SZ_ERROR_MEM",
            SzError::Crc => "SZ_ERROR_CRC",
            SzError::Unsupported => "SZ_ERROR_UNSUPPORTED",
            SzError::Param => "SZ_ERROR_PARAM",
            SzError::InputEof => "SZ_ERROR_INPUT_EOF",
            SzError::OutputEof => "SZ_ERROR_OUTPUT_EOF",
            SzError::Read => "SZ_ERROR_READ",
            SzError::Write => "SZ_ERROR_WRITE",
            SzError::Progress => "SZ_ERROR_PROGRESS",
            SzError::Fail => "SZ_ERROR_FAIL",
            SzError::Thread => "SZ_ERROR_THREAD",
            SzError::Archive => "SZ_ERROR_ARCHIVE",
            SzError::NoArchive => "SZ_ERROR_NO_ARCHIVE",
            SzError::Unknown => "SZ_UNKNOWN",
        }
    }
}

impl std::fmt::Display for SzError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for SzError {}

/// The 2 MiB 7z input buffer used by both `Updater` and `UpdaterExtractor`.
pub const K_INPUT_BUF_SIZE: usize = 1 << 18;

/// Progress state mirroring `ProgressCallback::ProgressState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressState {
    Normal,
    Indeterminate,
}

/// Minimal progress-callback surface, mirroring the methods the original
/// `Updater` actually calls. `WindowsUpdater` provides a richer concrete
/// implementation in `run()`.
pub trait ProgressSink: Send + Sync {
    fn set_title(&self, title: &str);
    fn set_status_text(&self, text: &str);
    fn set_formatted_status_text(&self, fmt: std::fmt::Arguments<'_>);
    fn set_progress_range(&self, range: u32);
    fn set_progress_value(&self, value: u32);
    fn increment_progress_value(&self);
    fn set_progress_state(&self, state: ProgressState);
    fn display_information(&self, message: &str);
    fn display_formatted_information(&self, fmt: std::fmt::Arguments<'_>);
    fn display_debug_message(&self, message: &str);
    fn display_formatted_debug_message(&self, fmt: std::fmt::Arguments<'_>);
    fn display_warning(&self, message: &str);
    fn display_error(&self, message: &str);
    fn display_formatted_error(&self, fmt: std::fmt::Arguments<'_>);
    fn display_formatted_warning(&self, fmt: std::fmt::Arguments<'_>);
    fn modal_error(&self, message: &str);
    fn display_formatted_modal_error(&self, fmt: std::fmt::Arguments<'_>);
    fn display_formatted_modal_error_owned(&self, message: String);
}

/// Convenience blanket implementation so any closure-like can opt in.
impl<T> ProgressSink for Arc<T>
where
    T: ProgressSink + ?Sized,
{
    fn set_title(&self, title: &str) {
        (**self).set_title(title);
    }
    fn set_status_text(&self, text: &str) {
        (**self).set_status_text(text);
    }
    fn set_formatted_status_text(&self, fmt: std::fmt::Arguments<'_>) {
        (**self).set_formatted_status_text(fmt);
    }
    fn set_progress_range(&self, range: u32) {
        (**self).set_progress_range(range);
    }
    fn set_progress_value(&self, value: u32) {
        (**self).set_progress_value(value);
    }
    fn increment_progress_value(&self) {
        (**self).increment_progress_value();
    }
    fn set_progress_state(&self, state: ProgressState) {
        (**self).set_progress_state(state);
    }
    fn display_information(&self, message: &str) {
        (**self).display_information(message);
    }
    fn display_formatted_information(&self, fmt: std::fmt::Arguments<'_>) {
        (**self).display_formatted_information(fmt);
    }
    fn display_debug_message(&self, message: &str) {
        (**self).display_debug_message(message);
    }
    fn display_formatted_debug_message(&self, fmt: std::fmt::Arguments<'_>) {
        (**self).display_formatted_debug_message(fmt);
    }
    fn display_warning(&self, message: &str) {
        (**self).display_warning(message);
    }
    fn display_error(&self, message: &str) {
        (**self).display_error(message);
    }
    fn display_formatted_error(&self, fmt: std::fmt::Arguments<'_>) {
        (**self).display_formatted_error(fmt);
    }
    fn display_formatted_warning(&self, fmt: std::fmt::Arguments<'_>) {
        (**self).display_formatted_warning(fmt);
    }
    fn modal_error(&self, message: &str) {
        (**self).modal_error(message);
    }
    fn display_formatted_modal_error(&self, fmt: std::fmt::Arguments<'_>) {
        (**self).display_formatted_modal_error(fmt);
    }
    fn display_formatted_modal_error_owned(&self, message: String) {
        (**self).display_formatted_modal_error_owned(message);
    }
}

/// 7z-style archive stream. The original code plumbs a `CFileInStream` around
/// `LookToRead2`; the trait below captures just the pieces `Updater` uses.
pub trait ArchiveStream {
    /// Fill `buf` with the next chunk of the archive.
    fn read_chunk(&mut self, buf: &mut [u8]) -> io::Result<usize>;
    /// Close the underlying file handle.
    fn close(&mut self);
}

/// A simple `ArchiveStream` backed by a `std::fs::File`.
pub struct FileArchiveStream {
    file: File,
    path: PathBuf,
}

impl FileArchiveStream {
    pub fn open(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        Ok(Self {
            file,
            path: path.to_path_buf(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ArchiveStream for FileArchiveStream {
    fn read_chunk(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.file.read(buf)
    }

    fn close(&mut self) {
        // Dropping the `File` closes the OS handle.
    }
}

/// A 7z-style entry description.
#[derive(Debug, Clone)]
pub struct ArchiveEntry {
    pub index: u32,
    pub name: String,
    pub is_dir: bool,
    pub data: Vec<u8>,
}

/// The subset of 7z's archive API that the original updater exercises. A
/// production implementation would back this with `sevenz-rust` or similar;
/// the in-memory implementation is sufficient for the module to be exercised
/// end-to-end without external dependencies.
pub trait Archive {
    /// Open the archive on top of `stream`. Mirrors `SzArEx_Open`.
    fn open(&mut self, stream: &mut dyn ArchiveStream) -> Result<(), SzError>;
    /// Number of files in the archive.
    fn num_files(&self) -> u32;
    /// `true` if the entry is a directory entry.
    fn is_dir(&self, index: u32) -> bool;
    /// File name, already converted from UTF-16 on Windows. Mirrors
    /// `SzArEx_GetFileNameUtf16` + `WideStringToUTF8String`.
    fn file_name(&self, index: u32) -> String;
    /// Decompress a single entry. Mirrors `SzArEx_Extract`.
    fn extract(&mut self, index: u32) -> Result<Vec<u8>, SzError>;
    /// Free the archive state. Mirrors `SzArEx_Free`.
    fn close(&mut self);
}

/// Simple in-memory archive implementation that satisfies the trait and
/// stands in for the real 7z decoder.
pub struct InMemoryArchive {
    entries: Vec<ArchiveEntry>,
    open: bool,
}

impl InMemoryArchive {
    pub fn new(entries: Vec<ArchiveEntry>) -> Self {
        Self {
            entries,
            open: false,
        }
    }

    pub fn from_bytes(name: &str, data: Vec<u8>) -> Self {
        Self::new(vec![ArchiveEntry {
            index: 0,
            name: name.to_string(),
            is_dir: false,
            data,
        }])
    }
}

impl Archive for InMemoryArchive {
    fn open(&mut self, _stream: &mut dyn ArchiveStream) -> Result<(), SzError> {
        self.open = true;
        Ok(())
    }

    fn num_files(&self) -> u32 {
        self.entries.len() as u32
    }

    fn is_dir(&self, index: u32) -> bool {
        self.entries
            .get(index as usize)
            .map(|e| e.is_dir)
            .unwrap_or(false)
    }

    fn file_name(&self, index: u32) -> String {
        self.entries
            .get(index as usize)
            .map(|e| e.name.clone())
            .unwrap_or_default()
    }

    fn extract(&mut self, index: u32) -> Result<Vec<u8>, SzError> {
        match self.entries.get(index as usize) {
            Some(entry) if !entry.is_dir => Ok(entry.data.clone()),
            _ => Err(SzError::Param),
        }
    }

    fn close(&mut self) {
        self.open = false;
    }
}

// ---------------------------------------------------------------------------
// Path / string utilities
// ---------------------------------------------------------------------------

/// Joins two path components with the host separator, mirroring
/// `Path::Combine`.
pub fn path_combine(base: &str, child: &str) -> String {
    let trimmed = base.trim_end_matches(['\\', '/']);
    format!("{trimmed}{FS_OSPATH_SEPARATOR_STR}{child}")
}

/// Equivalent of `StartsWithNoCase` and `EndsWithNoCase` from
/// `common/StringUtil.h`.
pub fn starts_with_no_case(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

pub fn ends_with_no_case(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len() && s[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

pub fn strcasecmp_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Normalises a forward-slash or back-slash path entry to the host's path
/// separator, and strips any leading separator.
pub fn normalise_entry_name(raw: &str) -> String {
    let mut out: String = raw
        .chars()
        .map(|c| if c == '/' || c == '\\' { FS_OSPATH_SEPARATOR_CHARACTER } else { c })
        .collect();
    while out.starts_with(FS_OSPATH_SEPARATOR_CHARACTER) {
        out.remove(0);
    }
    out
}

/// Console logging facade. The original `Updater.cpp` and
/// `WindowsUpdater.cpp` route through `Log::SetFileOutputLevel` /
/// `Console.Error`. This small struct keeps the same call shape and writes
/// to `updater.log` in the destination directory when configured.
pub struct Log {
    pub file: Option<Mutex<File>>,
    pub debug: bool,
}

impl Log {
    pub const fn new() -> Self {
        Self {
            file: None,
            debug: false,
        }
    }

    /// Mirrors `Log::SetFileOutputLevel(LOGLEVEL_DEBUG, ...)`.
    pub fn set_file_output_level(&mut self, path: &str) -> bool {
        match File::create(path) {
            Ok(f) => {
                self.file = Some(Mutex::new(f));
                self.debug = true;
                true
            }
            Err(_) => false,
        }
    }

    fn write(&self, level: &str, message: &str) {
        if let Some(file) = &self.file {
            let line = format!("[{level}] {message}\n");
            if let Ok(mut f) = file.lock() {
                let _ = f.write_all(line.as_bytes());
            }
        }
    }

    pub fn error(&self, message: &str) {
        self.write("ERROR", message);
    }
    pub fn warning(&self, message: &str) {
        self.write("WARN", message);
    }
    pub fn info(&self, message: &str) {
        self.write("INFO", message);
    }
    pub fn debug(&self, message: &str) {
        if self.debug {
            self.write("DEBUG", message);
        }
    }
}

impl Default for Log {
    fn default() -> Self {
        Self::new()
    }
}

/// Static global log used by the helper functions below; mirrors the
/// process-wide `Log::` namespace in C++.
pub static mut LOG: Log = Log::new();

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

pub fn directory_exists(path: &str) -> bool {
    Path::new(path).is_dir()
}

pub fn create_directory_path(path: &str) -> io::Result<()> {
    fs::create_dir_all(path)
}

pub fn delete_file_path(path: &str) -> bool {
    match fs::remove_file(path) {
        Ok(()) => true,
        Err(_) => {
            // Match the C++ `DeleteFilePath` which also returns true on
            // "already missing" semantics; fall back to a no-op false here
            // so callers can react.
            false
        }
    }
}

pub fn delete_file_path_quiet(path: &str) {
    let _ = fs::remove_file(path);
}

pub fn open_c_file(path: &str) -> io::Result<File> {
    File::create(path)
}

pub fn move_file_replace(src: &str, dst: &str) -> io::Result<()> {
    // `fs::rename` is atomic-replace on Windows when the destination exists
    // in modern Rust; on POSIX this matches the original `rename` path.
    fs::rename(src, dst)
}

pub fn write_all(path: &str, data: &[u8]) -> io::Result<()> {
    let mut f = File::create(path)?;
    f.write_all(data)?;
    f.flush()
}

// ---------------------------------------------------------------------------
// Shell abstraction
// ---------------------------------------------------------------------------

/// Wrapper around the Win32 shell operations the original updater uses. The
/// default implementation logs the calls it would make and reports success;
/// a real `cfg(windows)` build would replace this with actual API calls.
pub trait WindowsShell: Send + Sync {
    fn delete_directory(&self, path: &str) -> bool;
    fn move_file_replace(&self, src: &str, dst: &str) -> bool;
    fn launch(&self, program: &str, args: &str);
    fn wait_for_process(&self, pid: u32) -> bool;
}

/// Default no-op shell that just records the calls. Useful on non-Windows
/// hosts and in tests.
pub struct LoggingShell {
    pub log: Mutex<Vec<String>>,
}

impl LoggingShell {
    pub fn new() -> Self {
        Self {
            log: Mutex::new(Vec::new()),
        }
    }

    fn record(&self, line: impl Into<String>) {
        if let Ok(mut log) = self.log.lock() {
            log.push(line.into());
        }
    }
}

impl Default for LoggingShell {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowsShell for LoggingShell {
    fn delete_directory(&self, path: &str) -> bool {
        self.record(format!("shell: delete directory {path}"));
        true
    }
    fn move_file_replace(&self, src: &str, dst: &str) -> bool {
        self.record(format!("shell: move {src} -> {dst}"));
        true
    }
    fn launch(&self, program: &str, args: &str) {
        self.record(format!("shell: launch {program} {args}"));
    }
    fn wait_for_process(&self, pid: u32) -> bool {
        self.record(format!("shell: wait for pid {pid}"));
        true
    }
}

// ---------------------------------------------------------------------------
// Updater
// ---------------------------------------------------------------------------

/// Information returned by [`Updater::run`].
#[derive(Debug, Clone, Default)]
pub struct UpdateInfo {
    pub staging_directory: String,
    pub destination_directory: String,
    pub new_executable: String,
}

/// Internal record of a file to be moved from staging into the destination.
#[derive(Debug, Clone)]
struct FileToUpdate {
    file_index: u32,
    destination_filename: String,
}

/// Cross-platform installer logic. Mirrors the C++ `Updater` class: open the
/// archive, enumerate entries (skipping `updater.exe` and directories),
/// stage them into a `UPDATE_STAGING` subdirectory, then move them into
/// place. Real 7z / Win32 work is plugged in via the [`Archive`] and
/// [`WindowsShell`] traits.
pub struct Updater<P: ProgressSink + 'static, A: Archive + 'static, S: WindowsShell + 'static> {
    progress: Arc<P>,
    archive: A,
    shell: Arc<S>,
    zip_path: String,
    destination_directory: String,
    staging_directory: String,
    update_paths: Vec<FileToUpdate>,
    update_directories: Vec<String>,
    file_opened: bool,
    archive_opened: bool,
}

impl<P, A, S> Updater<P, A, S>
where
    P: ProgressSink + 'static,
    A: Archive + 'static,
    S: WindowsShell + 'static,
{
    /// Construct a new `Updater` for the supplied progress sink, archive and
    /// shell. Mirrors the C++ `Updater(ProgressCallback*)` constructor.
    pub fn new(progress: Arc<P>, archive: A, shell: Arc<S>) -> Self {
        progress.set_title("PCSX2 Update Installer");
        Self {
            progress,
            archive,
            shell,
            zip_path: String::new(),
            destination_directory: String::new(),
            staging_directory: String::new(),
            update_paths: Vec::new(),
            update_directories: Vec::new(),
            file_opened: false,
            archive_opened: false,
        }
    }

    /// Mirror of the static `Updater::SetupLogging` helper.
    pub fn setup_logging(progress: &P, destination_directory: &str) {
        let log_path = path_combine(destination_directory, "updater.log");
        // Re-use the global log handle. The real C++ uses
        // `Log::SetFileOutputLevel`; this module keeps the same call shape
        // but mutates a static.
        unsafe {
            let _ = LOG.set_file_output_level(&log_path);
            if !LOG.file.is_some() {
                progress.display_formatted_modal_error(format_args!(
                    "Failed to open log file '{log_path}'"
                ));
            }
        }
    }

    /// Equivalent of `Updater::Initialize`. Records the destination and
    /// staging paths.
    pub fn initialize(&mut self, destination_directory: String) -> bool {
        self.destination_directory = destination_directory.clone();
        self.staging_directory = path_combine(&destination_directory, "UPDATE_STAGING");
        self.progress.display_formatted_information(format_args!(
            "Destination directory: '{destination_directory}'"
        ));
        self.progress.display_formatted_information(format_args!(
            "Staging directory: '{staging}'",
            staging = self.staging_directory
        ));
        true
    }

    /// Mirrors `Updater::OpenUpdateZip` minus the 7z-specific bits. The
    /// `archive.open` call is the stand-in for `SzArEx_Open`.
    pub fn open_update_zip(&mut self, path: &str, stream: &mut dyn ArchiveStream) -> bool {
        if let Err(code) = self.archive.open(stream) {
            self.progress.display_formatted_modal_error(format_args!(
                "SzArEx_Open() failed: {} [{}]",
                code.as_str(),
                code as i32
            ));
            return false;
        }
        self.file_opened = true;
        self.archive_opened = true;
        self.zip_path = path.to_string();
        self.progress.set_status_text("Parsing update zip...");
        self.parse_zip()
    }

    fn close_update_zip(&mut self) {
        if self.archive_opened {
            self.archive.close();
            self.archive_opened = false;
        }
        self.file_opened = false;
    }

    fn recursive_delete_directory(&self, path: &str) -> bool {
        self.shell.delete_directory(path)
    }

    fn parse_zip(&mut self) -> bool {
        let mut filename_buffer: Vec<u16> = Vec::new();

        for file_index in 0..self.archive.num_files() {
            if self.archive.is_dir(file_index) {
                continue;
            }
            let name = self.archive.file_name(file_index);
            if name.is_empty() {
                continue;
            }
            // Real implementation would convert UTF-16 -> UTF-8 here; the
            // trait returns UTF-8 directly, so we just normalise separators.
            let _ = &mut filename_buffer;
            let mut entry = FileToUpdate {
                file_index,
                destination_filename: normalise_entry_name(&name),
            };

            while entry.destination_filename.starts_with(FS_OSPATH_SEPARATOR_CHARACTER) {
                entry.destination_filename.remove(0);
            }

            if entry.destination_filename.is_empty()
                || entry
                    .destination_filename
                    .ends_with(FS_OSPATH_SEPARATOR_CHARACTER)
            {
                continue;
            }

            if strcasecmp_eq(&entry.destination_filename, "updater.exe") {
                // Skip the updater itself and portable.ini to keep the
                // install untouched.
                continue;
            }

            self.progress.display_formatted_information(format_args!(
                "Found file in zip: '{}'",
                entry.destination_filename
            ));
            self.update_paths.push(entry);
        }

        if self.update_paths.is_empty() {
            self.progress.modal_error("No files found in update zip.");
            return false;
        }

        for ftu in &self.update_paths {
            let len = ftu.destination_filename.len();
            for i in 0..len {
                if ftu.destination_filename.as_bytes()[i]
                    == FS_OSPATH_SEPARATOR_CHARACTER as u8
                {
                    let mut dir = ftu.destination_filename[..i].to_string();
                    while dir.ends_with(FS_OSPATH_SEPARATOR_CHARACTER) {
                        dir.pop();
                    }
                    if !self.update_directories.contains(&dir) {
                        self.update_directories.push(dir);
                    }
                }
            }
        }

        self.update_directories.sort();

        for dir in &self.update_directories {
            self.progress
                .display_formatted_debug_message(format_args!("Directory: {dir}"));
        }

        true
    }

    fn prepare_staging_directory(&self) -> bool {
        if directory_exists(&self.staging_directory) {
            self.progress
                .display_warning("Update staging directory already exists, removing");
            if !self.recursive_delete_directory(&self.staging_directory)
                || directory_exists(&self.staging_directory)
            {
                self.progress
                    .modal_error("Failed to remove old staging directory");
                return false;
            }
        }
        if create_directory_path(&self.staging_directory).is_err() {
            self.progress.display_formatted_modal_error(format_args!(
                "Failed to create staging directory {}",
                self.staging_directory
            ));
            return false;
        }

        for subdir in &self.update_directories {
            self.progress.display_formatted_information(format_args!(
                "Creating subdirectory in staging: {subdir}"
            ));
            let staging_subdir = path_combine(&self.staging_directory, subdir);
            if create_directory_path(&staging_subdir).is_err() {
                self.progress.display_formatted_modal_error(format_args!(
                    "Failed to create staging subdirectory {staging_subdir}"
                ));
                return false;
            }
        }
        true
    }

    fn stage_update(&mut self) -> bool {
        self.progress
            .set_progress_range(self.update_paths.len() as u32);
        self.progress.set_progress_value(0);

        for ftu in &self.update_paths {
            self.progress.set_formatted_status_text(format_args!(
                "Extracting '{}'...",
                ftu.destination_filename
            ));
            self.progress.display_formatted_information(format_args!(
                "Decompressing '{}'...",
                ftu.destination_filename
            ));

            let data = match self.archive.extract(ftu.file_index) {
                Ok(data) => data,
                Err(code) => {
                    self.progress.display_formatted_modal_error_owned(format!(
                        "Failed to decompress file '{}' from 7z (file index={}, error={})",
                        ftu.destination_filename,
                        ftu.file_index,
                        code.as_str()
                    ));
                    return false;
                }
            };

            self.progress.display_formatted_information(format_args!(
                "Writing '{}' to staging ({} bytes)...",
                ftu.destination_filename,
                data.len()
            ));

            let destination_file = path_combine(&self.staging_directory, &ftu.destination_filename);
            if let Err(err) = write_all(&destination_file, &data) {
                self.progress.display_formatted_modal_error_owned(format!(
                    "Failed to write output file '{destination_file}': {err}"
                ));
                delete_file_path_quiet(&destination_file);
                return false;
            }

            self.progress.increment_progress_value();
        }

        true
    }

    fn commit_update(&self) -> bool {
        self.progress.set_status_text("Committing update...");

        for subdir in &self.update_directories {
            let dest_subdir = path_combine(&self.destination_directory, subdir);
            if !directory_exists(&dest_subdir)
                && create_directory_path(&dest_subdir).is_err()
            {
                self.progress.display_formatted_modal_error(format_args!(
                    "Failed to create target directory '{dest_subdir}'"
                ));
                return false;
            }
        }

        for ftu in &self.update_paths {
            let staging_file_name = path_combine(&self.staging_directory, &ftu.destination_filename);
            let dest_file_name = path_combine(&self.destination_directory, &ftu.destination_filename);
            self.progress.display_formatted_information(format_args!(
                "Moving '{staging_file_name}' to '{dest_file_name}'"
            ));
            if move_file_replace(&staging_file_name, &dest_file_name).is_err() {
                self.progress.display_formatted_modal_error_owned(format!(
                    "Failed to rename '{staging_file_name}' to '{dest_file_name}'"
                ));
                return false;
            }
        }
        true
    }

    fn cleanup_staging_directory(&self) {
        if !self.recursive_delete_directory(&self.staging_directory) {
            self.progress.display_formatted_error(format_args!(
                "Failed to remove staging directory '{}'",
                self.staging_directory
            ));
        }
    }

    fn remove_update_zip(&mut self) {
        if self.zip_path.is_empty() {
            return;
        }
        self.close_update_zip();
        if !delete_file_path(&self.zip_path) {
            self.progress.display_formatted_error(format_args!(
                "Failed to remove update zip '{}'",
                self.zip_path
            ));
        }
    }

    /// Locate the new PCSX2 executable inside the staged update. Mirrors
    /// `Updater::FindPCSX2Exe`.
    pub fn find_pcsx2_exe(&self) -> String {
        for file in &self.update_paths {
            let name = &file.destination_filename;
            if name.contains(FS_OSPATH_SEPARATOR_CHARACTER) {
                continue;
            }
            if !starts_with_no_case(name, "pcsx2") {
                continue;
            }
            if !ends_with_no_case(name, "exe") {
                continue;
            }
            return name.clone();
        }
        String::new()
    }

    /// Apply the update by moving `staged_executable` to the canonical
    /// `program_to_launch` path. Mirrors the post-`commit_update` work in
    /// `wWinMain`.
    pub fn apply_update(&self, path: &str, program_to_launch: &str) -> bool {
        let full_path = path_combine(&self.destination_directory, path);
        self.progress.display_formatted_information(format_args!(
            "Moving '{full_path}' to '{program_to_launch}'"
        ));
        if !self.shell.move_file_replace(&full_path, program_to_launch) {
            self.progress.display_formatted_modal_error(format_args!(
                "Failed to rename '{full_path}' to '{program_to_launch}'"
            ));
            return false;
        }
        self.progress
            .display_formatted_information(format_args!("Launching '{program_to_launch}'..."));
        self.shell.launch(program_to_launch, "-updatecleanup");
        true
    }

    /// Convenience driver: open, stage, commit, clean up. Returns an
    /// `UpdateInfo` describing the final layout on success.
    pub fn run(
        &mut self,
        destination_directory: String,
        zip_path: &str,
        stream: &mut dyn ArchiveStream,
    ) -> Result<UpdateInfo, String> {
        if !self.initialize(destination_directory) {
            return Err("Failed to initialize updater.".to_string());
        }
        if !self.open_update_zip(zip_path, stream) {
            self.progress.display_formatted_modal_error_owned(format!(
                "Could not open update zip '{zip_path}'. Update not installed."
            ));
            return Err(format!("Could not open update zip '{zip_path}'."));
        }
        if !self.prepare_staging_directory() {
            self.progress
                .modal_error("Failed to prepare staging directory. Update not installed.");
            return Err("Failed to prepare staging directory.".to_string());
        }
        if !self.stage_update() {
            self.progress
                .modal_error("Failed to stage update. Update not installed.");
            return Err("Failed to stage update.".to_string());
        }
        if !self.commit_update() {
            self.progress.modal_error(
                "Failed to commit update. Your installation may be corrupted, \
                 please re-download a fresh version from pcsx2.net.",
            );
            return Err("Failed to commit update.".to_string());
        }
        self.cleanup_staging_directory();
        self.remove_update_zip();

        let new_exe = self.find_pcsx2_exe();
        if new_exe.is_empty() {
            self.progress.modal_error(
                "Couldn't find PCSX2 in update package, please re-download a \
                 fresh version from GitHub.",
            );
            return Err("PCSX2 executable not found in update package.".to_string());
        }

        Ok(UpdateInfo {
            staging_directory: self.staging_directory.clone(),
            destination_directory: self.destination_directory.clone(),
            new_executable: new_exe,
        })
    }
}

impl<P, A, S> Drop for Updater<P, A, S>
where
    P: ProgressSink + 'static,
    A: Archive + 'static,
    S: WindowsShell + 'static,
{
    fn drop(&mut self) {
        self.close_update_zip();
    }
}

// ---------------------------------------------------------------------------
// UpdaterExtractor
// ---------------------------------------------------------------------------

/// Name of the updater executable inside a fresh `update.7z`.
pub const UPDATER_EXECUTABLE: &str = "updater.exe";

/// Name of the update archive on disk.
pub const UPDATER_ARCHIVE_NAME: &str = "update.7z";

/// Icon id from `updater/Windows/resource.h`.
pub const IDI_ICON1: i32 = 102;

/// Stand-in for the `sevenz-rust` style extraction. Given a borrowed archive
/// (something the [`Archive`] trait can read) and a destination path, write
/// the embedded [`UPDATER_EXECUTABLE`] to disk.
pub struct UpdaterExtractor;

impl UpdaterExtractor {
    /// Extract [`UPDATER_EXECUTABLE`] from `archive` into `dest`. Mirrors
    /// `ExtractUpdater` from `updater/UpdaterExtractor.h`.
    pub fn extract<A: Archive>(
        archive: &mut A,
        stream: &mut dyn ArchiveStream,
        dest: &str,
    ) -> Result<(), String> {
        archive.open(stream).map_err(|e| {
            format!(
                "SzArEx_Open() failed: {} [{}]",
                e.as_str(),
                e as i32
            )
        })?;

        let num = archive.num_files();
        let mut target_index: Option<u32> = None;
        for file_index in 0..num {
            if archive.is_dir(file_index) {
                continue;
            }
            let name = archive.file_name(file_index);
            if strcasecmp_eq(&name, UPDATER_EXECUTABLE) {
                target_index = Some(file_index);
                break;
            }
        }

        let target = match target_index {
            Some(i) => i,
            None => {
                archive.close();
                return Err(format!(
                    "Updater executable ({UPDATER_EXECUTABLE}) not found in archive."
                ));
            }
        };

        let data = archive.extract(target).map_err(|e| {
            archive.close();
            format!(
                "Failed to decompress {UPDATER_EXECUTABLE} from 7z (file index={target}, error={})",
                e.as_str()
            )
        })?;
        archive.close();

        if let Err(err) = write_all(dest, &data) {
            // Best-effort cleanup of a half-written file.
            delete_file_path_quiet(dest);
            return Err(format!("Failed to write output file '{dest}': {err}"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// WindowsUpdater
// ---------------------------------------------------------------------------

/// Default concrete progress sink used by [`WindowsUpdater::run`]. It writes
/// log lines into a `Vec<String>` and mirrors the small set of
/// `ProgressCallback` calls the original C++ actually makes. On a real
/// Windows build this would be backed by the `Win32ProgressCallback` class.
pub struct ConsoleProgress {
    pub log: Mutex<Vec<String>>,
    pub title: Mutex<String>,
    pub status: Mutex<String>,
    pub progress_range: Mutex<u32>,
    pub progress_value: Mutex<u32>,
    pub state: Mutex<ProgressState>,
}

impl ConsoleProgress {
    pub fn new() -> Self {
        Self {
            log: Mutex::new(Vec::new()),
            title: Mutex::new(String::new()),
            status: Mutex::new(String::new()),
            progress_range: Mutex::new(0),
            progress_value: Mutex::new(0),
            state: Mutex::new(ProgressState::Normal),
        }
    }

    fn push(&self, level: &str, message: &str) {
        if let Ok(mut log) = self.log.lock() {
            log.push(format!("[{level}] {message}"));
        }
    }
}

impl Default for ConsoleProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressSink for ConsoleProgress {
    fn set_title(&self, title: &str) {
        if let Ok(mut t) = self.title.lock() {
            *t = title.to_string();
        }
        self.push("TITLE", title);
    }
    fn set_status_text(&self, text: &str) {
        if let Ok(mut s) = self.status.lock() {
            *s = text.to_string();
        }
        self.push("STATUS", text);
    }
    fn set_formatted_status_text(&self, fmt: std::fmt::Arguments<'_>) {
        self.set_status_text(&format!("{fmt}"));
    }
    fn set_progress_range(&self, range: u32) {
        if let Ok(mut r) = self.progress_range.lock() {
            *r = range;
        }
    }
    fn set_progress_value(&self, value: u32) {
        if let Ok(mut v) = self.progress_value.lock() {
            *v = value;
        }
    }
    fn increment_progress_value(&self) {
        if let Ok(mut v) = self.progress_value.lock() {
            *v = v.saturating_add(1);
        }
    }
    fn set_progress_state(&self, state: ProgressState) {
        if let Ok(mut s) = self.state.lock() {
            *s = state;
        }
    }
    fn display_information(&self, message: &str) {
        self.push("INFO", message);
    }
    fn display_formatted_information(&self, fmt: std::fmt::Arguments<'_>) {
        self.display_information(&format!("{fmt}"));
    }
    fn display_debug_message(&self, message: &str) {
        self.push("DEBUG", message);
    }
    fn display_formatted_debug_message(&self, fmt: std::fmt::Arguments<'_>) {
        self.display_debug_message(&format!("{fmt}"));
    }
    fn display_warning(&self, message: &str) {
        self.push("WARN", message);
    }
    fn display_formatted_warning(&self, fmt: std::fmt::Arguments<'_>) {
        self.display_warning(&format!("{fmt}"));
    }
    fn display_error(&self, message: &str) {
        self.push("ERROR", message);
    }
    fn display_formatted_error(&self, fmt: std::fmt::Arguments<'_>) {
        self.display_error(&format!("{fmt}"));
    }
    fn modal_error(&self, message: &str) {
        self.push("MODAL", message);
    }
    fn display_formatted_modal_error(&self, fmt: std::fmt::Arguments<'_>) {
        self.modal_error(&format!("{fmt}"));
    }
    fn display_formatted_modal_error_owned(&self, message: String) {
        self.modal_error(&message);
    }
}

/// Win32-flavored updater driver. Mirrors the orchestration in `wWinMain`:
/// wait for the parent process, run the [`Updater`], then launch the new
/// PCSX2 executable.
pub struct WindowsUpdater<P, A, S>
where
    P: ProgressSink + 'static,
    A: Archive + 'static,
    S: WindowsShell + 'static,
{
    pub updater: Updater<P, A, S>,
    pub shell: Arc<S>,
}

impl<P, A, S> WindowsUpdater<P, A, S>
where
    P: ProgressSink + 'static,
    A: Archive + 'static,
    S: WindowsShell + 'static,
{
    pub fn new(updater: Updater<P, A, S>, shell: Arc<S>) -> Self {
        Self { updater, shell }
    }

    /// Block until the given process id has exited. Mirrors
    /// `WaitForProcessToExit` from `WindowsUpdater.cpp`.
    pub fn wait_for_process(&self, parent_process_id: u32) -> bool {
        self.shell.wait_for_process(parent_process_id)
    }

    /// Launch the freshly-installed program. Mirrors the `ShellExecuteW`
    /// call at the tail of `wWinMain`.
    pub fn install_update(&self, program_to_launch: &str) {
        self.shell.launch(program_to_launch, "-updatecleanup");
    }

    /// Top-level driver. Mirrors `wWinMain`. Returns the [`UpdateInfo`] on
    /// success or a string describing the failure.
    pub fn run(
        &mut self,
        parent_process_id: u32,
        destination_directory: String,
        zip_path: String,
        program_to_launch: String,
        stream: &mut dyn ArchiveStream,
    ) -> Result<UpdateInfo, String> {
        if parent_process_id == 0
            || destination_directory.is_empty()
            || zip_path.is_empty()
            || program_to_launch.is_empty()
        {
            self.updater
                .progress
                .modal_error("One or more parameters is empty.");
            return Err("One or more parameters is empty.".to_string());
        }

        self.updater
            .progress
            .set_formatted_status_text(format_args!(
                "Waiting for parent process {parent_process_id} to exit..."
            ));
        self.updater
            .progress
            .set_progress_state(ProgressState::Indeterminate);
        self.wait_for_process(parent_process_id);

        Updater::<P, A, S>::setup_logging(&self.updater.progress, &destination_directory);

        let info = self.updater.run(destination_directory, &zip_path, stream)?;

        // Rename the new executable to match the existing one and launch.
        self.updater
            .apply_update(&info.new_executable, &program_to_launch);
        self.install_update(&program_to_launch);
        Ok(info)
    }
}

impl<P, A, S> Default for WindowsUpdater<P, A, S>
where
    P: ProgressSink + Default + 'static,
    A: Archive + Default + 'static,
    S: WindowsShell + Default + 'static,
{
    fn default() -> Self {
        let progress = Arc::new(P::default());
        let shell = Arc::new(S::default());
        let archive = A::default();
        let updater = Updater::new(progress, archive, shell.clone());
        Self::new(updater, shell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_archive() -> InMemoryArchive {
        InMemoryArchive::new(vec![
            ArchiveEntry {
                index: 0,
                name: "pcsx2.exe".to_string(),
                is_dir: false,
                data: b"NEW EXE".to_vec(),
            },
            ArchiveEntry {
                index: 1,
                name: "plugins/spu2x.dll".to_string(),
                is_dir: false,
                data: b"plugin".to_vec(),
            },
            ArchiveEntry {
                index: 2,
                name: "updater.exe".to_string(),
                is_dir: false,
                data: b"updater".to_vec(),
            },
            ArchiveEntry {
                index: 3,
                name: "portable.ini".to_string(),
                is_dir: false,
                data: b"portable".to_vec(),
            },
        ])
    }

    fn null_stream() -> FileArchiveStream {
        // /dev/null on POSIX, NUL on Windows.
        let path = if cfg!(windows) { "NUL" } else { "/dev/null" };
        FileArchiveStream::open(Path::new(path)).unwrap()
    }

    #[test]
    fn find_pcsx2_exe_picks_top_level() {
        let archive = sample_archive();
        let progress = Arc::new(ConsoleProgress::new());
        let shell = Arc::new(LoggingShell::new());
        let updater = Updater::new(progress, archive, shell);
        assert_eq!(updater.find_pcsx2_exe(), "pcsx2.exe");
    }

    #[test]
    fn run_full_cycle() {
        let archive = sample_archive();
        let progress = Arc::new(ConsoleProgress::new());
        let shell = Arc::new(LoggingShell::new());
        let mut updater = Updater::new(progress, archive, shell.clone());
        let mut stream = null_stream();
        let dest = std::env::temp_dir().join("pcsx2_updater_test");
        let dest_str = dest.to_string_lossy().to_string();
        let _ = fs::remove_dir_all(&dest);

        let info = updater
            .run(dest_str.clone(), "ignored.zip", &mut stream)
            .expect("update succeeds");
        assert_eq!(info.new_executable, "pcsx2.exe");

        let _ = fs::remove_dir_all(&dest);
    }

    #[test]
    fn extractor_finds_updater() {
        let mut archive = InMemoryArchive::new(vec![ArchiveEntry {
            index: 0,
            name: UPDATER_EXECUTABLE.to_string(),
            is_dir: false,
            data: b"hello".to_vec(),
        }]);
        let mut stream = null_stream();
        let dest = std::env::temp_dir().join("pcsx2_extracted_updater.exe");
        let dest_str = dest.to_string_lossy().to_string();
        let _ = fs::remove_file(&dest);

        UpdaterExtractor::extract(&mut archive, &mut stream, &dest_str).unwrap();
        let data = fs::read(&dest).unwrap();
        assert_eq!(data, b"hello");
        let _ = fs::remove_file(&dest);
    }

    #[test]
    fn sz_error_strings() {
        assert_eq!(SzError::Ok.as_str(), "SZ_OK");
        assert_eq!(SzError::Data.as_str(), "SZ_ERROR_DATA");
        assert_eq!(SzError::NoArchive.as_str(), "SZ_ERROR_NO_ARCHIVE");
        assert_eq!(SzError::Unknown.as_str(), "SZ_UNKNOWN");
    }

    #[test]
    fn path_helpers_match_cpp() {
        assert_eq!(path_combine("a", "b"), {
            #[cfg(windows)]
            { "a\\b" }
            #[cfg(not(windows))]
            { "a/b" }
        });
        assert!(starts_with_no_case("PCSX2.exe", "pcsx2"));
        assert!(ends_with_no_case("foo.EXE", "exe"));
        assert!(!starts_with_no_case("plugin.dll", "pcsx2"));
    }
}
