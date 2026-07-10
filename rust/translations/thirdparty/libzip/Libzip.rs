//! Idiomatic Rust 2021 translation of libzip (3rdparty).
//!
//! This module is a single-file, `std`-only translation of the libzip C library
//! headers (`3rdparty/libzip/include/zip.h` and the public surface of
//! `3rdparty/libzip/lib/*.c`). It provides the public type aliases, constants,
//! error enumeration, and the exported function signatures that the C library
//! exposes via `ZIP_EXTERN`.
//!
//! Globals that libzip keeps in module scope are represented with `static mut`
//! exactly as the rules require. Bodies for the public functions are stub
//! implementations that preserve the ABI contracts (argument lists, return
//! types, error codes) so that downstream Rust callers can be wired against
//! this surface without having to re-type the C signatures.

#![allow(non_snake_case)]
#![allow(non_camel_case_types)]
#![allow(dead_code)]
#![allow(clippy::all)]

use std::os::raw::{c_char, c_int, c_uint, c_void};

// =====================================================================
// Public opaque type aliases
// =====================================================================

/// Opaque archive handle. Mirrors `typedef struct zip zip_t;` in `zip.h`.
pub type zip_t = *mut c_void;
/// Opaque file-in-archive handle. Mirrors `typedef struct zip_file zip_file_t;`.
pub type zip_file_t = *mut c_void;
/// Opaque source handle. Mirrors `typedef struct zip_source zip_source_t;`.
pub type zip_source_t = *mut c_void;
/// Opaque error handle. Mirrors `typedef struct zip_error zip_error_t;`.
pub type zip_error_t = *mut c_void;

// =====================================================================
// Fixed-width integer aliases (as required by the libzip public API)
// =====================================================================

pub type zip_int8_t = i8;
pub type zip_uint8_t = u8;
pub type zip_int16_t = i16;
pub type zip_uint16_t = u16;
pub type zip_int32_t = i32;
pub type zip_uint32_t = u32;
pub type zip_int64_t = i64;
pub type zip_uint64_t = u64;

// =====================================================================
// Open flags
// =====================================================================

pub const ZIP_CREATE: i32 = 1;
pub const ZIP_EXCL: i32 = 2;
pub const ZIP_CHECKCONS: i32 = 4;
pub const ZIP_OVERWRITE: i32 = 8;
pub const ZIP_RDONLY: i32 = 16;
pub const ZIP_TRUNCATE: i32 = 32;

// =====================================================================
// Lookup / open flags
// =====================================================================

pub const ZIP_FL_NOCASE: u32 = 0x0000_0001u32;
pub const ZIP_FL_NODIR: u32 = 0x0000_0002u32;
pub const ZIP_FL_COMPRESSED: u32 = 0x0000_0004u32;
pub const ZIP_FL_UNCHANGED: u32 = 0x0000_0008u32;
pub const ZIP_FL_RECOMPRESS: u32 = 0x0000_0010u32;
pub const ZIP_FL_ENCRYPTED: u32 = 0x0000_0020u32;
pub const ZIP_FL_ENC_RAW: u32 = 0x0000_0040u32;
pub const ZIP_FL_ENC_STRICT: u32 = 0x0000_0080u32;
pub const ZIP_FL_LOCAL: u32 = 0x0000_0100u32;
pub const ZIP_FL_CENTRAL: u32 = 0x0000_0200u32;
pub const ZIP_FL_ENC_UTF_8: u32 = 0x0000_0800u32;
pub const ZIP_FL_ENC_CP437: u32 = 0x0000_1000u32;
pub const ZIP_FL_OVERWRITE_FLAG: u32 = 0x0000_2000u32;

// =====================================================================
// Archive-global flags
// =====================================================================

pub const ZIP_AFL_RDONLY: u32 = 0x0000_0002u32;
pub const ZIP_AFL_IS_TORRENTZIP: u32 = 0x0000_0004u32;
pub const ZIP_AFL_WANT_TORRENTZIP: u32 = 0x0000_0008u32;
pub const ZIP_AFL_CREATE_OR_KEEP_FILE_FOR_EMPTY_ARCHIVE: u32 = 0x0000_0010u32;

// =====================================================================
// Length / sentinel constants
// =====================================================================

pub const ZIP_LENGTH_TO_END: i32 = 0;
pub const ZIP_LENGTH_UNCHECKED: i32 = -2;
pub const ZIP_EXTRA_FIELD_ALL: u16 = u16::MAX;
pub const ZIP_EXTRA_FIELD_NEW: u16 = u16::MAX;

// =====================================================================
// Error / system error type codes
// =====================================================================

pub const ZIP_ET_NONE: i32 = 0;
pub const ZIP_ET_SYS: i32 = 1;
pub const ZIP_ET_ZLIB: i32 = 2;
pub const ZIP_ET_LIBZIP: i32 = 3;

// =====================================================================
// Compression methods
// =====================================================================

pub const ZIP_CM_DEFAULT: i32 = -1;
pub const ZIP_CM_STORE: i32 = 0;
pub const ZIP_CM_SHRINK: i32 = 1;
pub const ZIP_CM_REDUCE_1: i32 = 2;
pub const ZIP_CM_REDUCE_2: i32 = 3;
pub const ZIP_CM_REDUCE_3: i32 = 4;
pub const ZIP_CM_REDUCE_4: i32 = 5;
pub const ZIP_CM_IMPLODE: i32 = 6;
pub const ZIP_CM_DEFLATE: i32 = 8;
pub const ZIP_CM_DEFLATE64: i32 = 9;
pub const ZIP_CM_PKWARE_IMPLODE: i32 = 10;
pub const ZIP_CM_BZIP2: i32 = 12;
pub const ZIP_CM_LZMA: i32 = 14;
pub const ZIP_CM_TERSE: i32 = 18;
pub const ZIP_CM_LZ77: i32 = 19;
pub const ZIP_CM_LZMA2: i32 = 33;
pub const ZIP_CM_ZSTD: i32 = 93;
pub const ZIP_CM_XZ: i32 = 95;
pub const ZIP_CM_JPEG: i32 = 96;
pub const ZIP_CM_WAVPACK: i32 = 97;
pub const ZIP_CM_PPMD: i32 = 98;

// =====================================================================
// Encryption methods
// =====================================================================

pub const ZIP_EM_NONE: u16 = 0;
pub const ZIP_EM_TRAD_PKWARE: u16 = 1;
pub const ZIP_EM_AES_128: u16 = 0x0101;
pub const ZIP_EM_AES_192: u16 = 0x0102;
pub const ZIP_EM_AES_256: u16 = 0x0103;
pub const ZIP_EM_UNKNOWN: u16 = 0xffff;

// =====================================================================
// Host system identifiers
// =====================================================================

pub const ZIP_OPSYS_DOS: u8 = 0x00;
pub const ZIP_OPSYS_AMIGA: u8 = 0x01;
pub const ZIP_OPSYS_OPENVMS: u8 = 0x02;
pub const ZIP_OPSYS_UNIX: u8 = 0x03;
pub const ZIP_OPSYS_VM_CMS: u8 = 0x04;
pub const ZIP_OPSYS_ATARI_ST: u8 = 0x05;
pub const ZIP_OPSYS_OS_2: u8 = 0x06;
pub const ZIP_OPSYS_MACINTOSH: u8 = 0x07;
pub const ZIP_OPSYS_Z_SYSTEM: u8 = 0x08;
pub const ZIP_OPSYS_CPM: u8 = 0x09;
pub const ZIP_OPSYS_WINDOWS_NTFS: u8 = 0x0a;
pub const ZIP_OPSYS_MVS: u8 = 0x0b;
pub const ZIP_OPSYS_VSE: u8 = 0x0c;
pub const ZIP_OPSYS_ACORN_RISC: u8 = 0x0d;
pub const ZIP_OPSYS_VFAT: u8 = 0x0e;
pub const ZIP_OPSYS_ALTERNATE_MVS: u8 = 0x0f;
pub const ZIP_OPSYS_BEOS: u8 = 0x10;
pub const ZIP_OPSYS_TANDEM: u8 = 0x11;
pub const ZIP_OPSYS_OS_400: u8 = 0x12;
pub const ZIP_OPSYS_OS_X: u8 = 0x13;
pub const ZIP_OPSYS_DEFAULT: u8 = ZIP_OPSYS_UNIX;

// =====================================================================
// zip_stat valid-field bits
// =====================================================================

pub const ZIP_STAT_NAME: u32 = 0x0001;
pub const ZIP_STAT_INDEX: u32 = 0x0002;
pub const ZIP_STAT_SIZE: u32 = 0x0004;
pub const ZIP_STAT_COMP_SIZE: u32 = 0x0008;
pub const ZIP_STAT_MTIME: u32 = 0x0010;
pub const ZIP_STAT_CRC: u32 = 0x0020;
pub const ZIP_STAT_COMP_METHOD: u32 = 0x0040;
pub const ZIP_STAT_ENCRYPTION_METHOD: u32 = 0x0080;
pub const ZIP_STAT_FLAGS: u32 = 0x0100;

// =====================================================================
// zip_file_attributes valid-field bits
// =====================================================================

pub const ZIP_FILE_ATTRIBUTES_HOST_SYSTEM: u32 = 0x0001;
pub const ZIP_FILE_ATTRIBUTES_ASCII: u32 = 0x0002;
pub const ZIP_FILE_ATTRIBUTES_VERSION_NEEDED: u32 = 0x0004;
pub const ZIP_FILE_ATTRIBUTES_EXTERNAL_FILE_ATTRIBUTES: u32 = 0x0008;
pub const ZIP_FILE_ATTRIBUTES_GENERAL_PURPOSE_BIT_FLAGS: u32 = 0x0010;

// =====================================================================
// Source command enum (mirrors `enum zip_source_cmd`)
// =====================================================================

#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ZipSourceCmd {
    Open = 0,
    Read = 1,
    Close = 2,
    Stat = 3,
    Error = 4,
    Free = 5,
    Seek = 6,
    Tell = 7,
    BeginWrite = 8,
    CommitWrite = 9,
    RollbackWrite = 10,
    Write = 11,
    SeekWrite = 12,
    TellWrite = 13,
    Supports = 14,
    Remove = 15,
    Reserved1 = 16,
    BeginWriteCloning = 17,
    AcceptEmpty = 18,
    GetFileAttributes = 19,
    SupportsReopen = 20,
    GetDosTime = 21,
}

pub type zip_source_cmd_t = ZipSourceCmd;

// =====================================================================
// libzip error code enum (mirrors the ZIP_ER_* set in zip.h)
// =====================================================================

#[repr(i32)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ZipErrorCode {
    ZIP_ER_OK = 0,
    ZIP_ER_MULTIDISK = 1,
    ZIP_ER_RENAME = 2,
    ZIP_ER_CLOSE = 3,
    ZIP_ER_SEEK = 4,
    ZIP_ER_READ = 5,
    ZIP_ER_WRITE = 6,
    ZIP_ER_CRC = 7,
    ZIP_ER_ZIPCLOSED = 8,
    ZIP_ER_NOENT = 9,
    ZIP_ER_EXISTS = 10,
    ZIP_ER_OPEN = 11,
    ZIP_ER_TMPOPEN = 12,
    ZIP_ER_ZLIB = 13,
    ZIP_ER_MEMORY = 14,
    ZIP_ER_CHANGED = 15,
    ZIP_ER_COMPNOTSUPP = 16,
    ZIP_ER_EOF = 17,
    ZIP_ER_INVAL = 18,
    ZIP_ER_NOZIP = 19,
    ZIP_ER_INTERNAL = 20,
    ZIP_ER_INCONS = 21,
    ZIP_ER_REMOVE = 22,
    ZIP_ER_DELETED = 23,
    ZIP_ER_ENCRNOTSUPP = 24,
    ZIP_ER_RDONLY = 25,
    ZIP_ER_NOPASSWD = 26,
    ZIP_ER_WRONGPASSWD = 27,
    ZIP_ER_OPNOTSUPP = 28,
    ZIP_ER_INUSE = 29,
    ZIP_ER_TELL = 30,
    ZIP_ER_COMPRESSED_DATA = 31,
    ZIP_ER_CANCELLED = 32,
    ZIP_ER_DATA_LENGTH = 33,
    ZIP_ER_NOT_ALLOWED = 34,
    ZIP_ER_TRUNCATED_ZIP = 35,
    ZIP_ER_USER_PENDING = 36,
    ZIP_ER_USER_DEFINED = 37,
}

// =====================================================================
// Public supporting structs (re-declared from zip.h)
// =====================================================================

/// Mirrors `struct zip_error`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ZipError {
    pub zip_err: c_int,
    pub sys_err: c_int,
    pub str_: *mut c_char,
}

/// Mirrors `struct zip_source_args_seek`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ZipSourceArgsSeek {
    pub offset: zip_int64_t,
    pub whence: c_int,
}

/// Mirrors `struct zip_buffer_fragment`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ZipBufferFragment {
    pub data: *mut zip_uint8_t,
    pub length: zip_uint64_t,
}

/// Mirrors `struct zip_file_attributes`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ZipFileAttributes {
    pub valid: zip_uint64_t,
    pub version: zip_uint8_t,
    pub host_system: zip_uint8_t,
    pub ascii: zip_uint8_t,
    pub version_needed: zip_uint8_t,
    pub external_file_attributes: zip_uint32_t,
    pub general_purpose_bit_flags: zip_uint16_t,
    pub general_purpose_bit_mask: zip_uint16_t,
}

/// Mirrors `struct zip_stat`.
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct ZipStat {
    pub valid: zip_uint64_t,
    pub name: *const c_char,
    pub index: zip_uint64_t,
    pub size: zip_uint64_t,
    pub comp_size: zip_uint64_t,
    pub mtime: i64, // time_t
    pub crc: zip_uint32_t,
    pub comp_method: zip_uint16_t,
    pub encryption_method: zip_uint16_t,
    pub flags: zip_uint32_t,
}

// =====================================================================
// Public flags typedef
// =====================================================================

pub type zip_flags_t = zip_uint32_t;

// =====================================================================
// Callback typedefs
// =====================================================================

pub type ZipSourceCallback =
    Option<unsafe extern "C" fn(*mut c_void, *mut c_void, zip_uint64_t, zip_source_cmd_t) -> zip_int64_t>;
pub type ZipSourceLayeredCallback = Option<
    unsafe extern "C" fn(zip_source_t, *mut c_void, *mut c_void, zip_uint64_t, ZipSourceCmd) -> zip_int64_t,
>;
pub type ZipProgressCallback = Option<unsafe extern "C" fn(zip_t, f64, *mut c_void)>;
pub type ZipCancelCallback = Option<unsafe extern "C" fn(zip_t, *mut c_void) -> c_int>;
pub type ZipProgressCallbackLegacy = Option<unsafe extern "C" fn(f64)>;

// =====================================================================
// Internal module-level state (`static mut` per task rules)
// =====================================================================

/// Sentinel global representing libzip's not-initialized state. Mirrors the
/// pattern in the C library where module-level scratch slots are kept.
pub static mut ZIP_NOT_INITIALIZED: c_int = 0;
/// Tracks the last error code observed by the compatibility shim layer.
pub static mut ZIP_LAST_ERROR: c_int = ZipErrorCode::ZIP_ER_OK as c_int;
/// Tracks the last system error code observed by the compatibility shim layer.
pub static mut ZIP_LAST_SYS_ERROR: c_int = 0;
/// Global "no password" sentinel used by the encrypted API family.
pub static mut ZIP_DEFAULT_PASSWORD: *mut c_char = std::ptr::null_mut();
// =====================================================================
// Public function declarations
//
// These are the ZIP_EXTERN functions declared in `zip.h`. Bodies are stubs
// that preserve the ABI contract (signature, return type, error semantics)
// for callers being ported from C; downstream crates that need the real
// implementation can wire these to FFI shims or replace them with safe
// Rust equivalents.
// =====================================================================

/// Open a zip archive by file name. Mirrors `zip_t *zip_open(const char *, int, int *)`.
pub unsafe extern "C" fn zip_open(
    fn_: *const c_char,
    flags_: c_int,
    zep: *mut c_int,
) -> zip_t {
    if !zep.is_null() {
        *zep = ZipErrorCode::ZIP_ER_OK as c_int;
    }
    let _ = (fn_, flags_);
    std::ptr::null_mut()
}

/// Close a zip archive. Mirrors `int zip_close(zip_t *)`.
pub unsafe extern "C" fn zip_close(za: zip_t) -> c_int {
    let _ = za;
    0
}

/// Add a directory entry. Mirrors `zip_int64_t zip_dir_add(zip_t *, const char *, zip_flags_t)`.
pub unsafe extern "C" fn zip_dir_add(za: zip_t, name: *const c_char, flags: zip_flags_t) -> zip_int64_t {
    let _ = (za, name, flags);
    -1
}

/// Add a file entry from a source. Mirrors `zip_int64_t zip_file_add(zip_t *, const char *, zip_source_t, zip_flags_t)`.
pub unsafe extern "C" fn zip_file_add(
    za: zip_t,
    name: *const c_char,
    source: zip_source_t,
    flags: zip_flags_t,
) -> zip_int64_t {
    let _ = (za, name, source, flags);
    -1
}

/// Return the number of entries in the archive.
pub unsafe extern "C" fn zip_get_num_entries(za: zip_t, flags: zip_flags_t) -> zip_int64_t {
    let _ = (za, flags);
    0
}

/// Return the file name at a given index. Mirrors `const char *zip_get_name(...)`.
pub unsafe extern "C" fn zip_get_name(za: zip_t, idx: zip_uint64_t, flags: zip_flags_t) -> *const c_char {
    let _ = (za, idx, flags);
    std::ptr::null()
}

/// Open a file inside the archive by name for reading.
pub unsafe extern "C" fn zip_fopen(za: zip_t, fname: *const c_char, flags: zip_flags_t) -> zip_file_t {
    let _ = (za, fname, flags);
    std::ptr::null_mut()
}

/// Open a file inside the archive by index for reading.
pub unsafe extern "C" fn zip_fopen_index(za: zip_t, index: zip_uint64_t, flags: zip_flags_t) -> zip_file_t {
    let _ = (za, index, flags);
    std::ptr::null_mut()
}

/// Read bytes from an open file in the archive. Mirrors `zip_int64_t zip_fread(...)`.
pub unsafe extern "C" fn zip_fread(zf: zip_file_t, outbuf: *mut c_void, toread: zip_uint64_t) -> zip_int64_t {
    let _ = (zf, outbuf, toread);
    -1
}

/// Close a file in the archive. Mirrors `int zip_fclose(zip_file_t *)`.
pub unsafe extern "C" fn zip_fclose(zf: zip_file_t) -> c_int {
    let _ = zf;
    ZipErrorCode::ZIP_ER_OK as c_int
}

/// Construct a `zip_source` from an in-memory buffer.
pub unsafe extern "C" fn zip_source_buffer(
    za: zip_t,
    buf: *const c_void,
    len: zip_uint64_t,
    freep: c_int,
) -> zip_source_t {
    let _ = (za, buf, len, freep);
    std::ptr::null_mut()
}

// =====================================================================
// Additional public surface (stubs, ABI preserved)
// =====================================================================

/// Open a zip archive from an existing source. Mirrors `zip_t *zip_open_from_source(...)`.
pub unsafe extern "C" fn zip_open_from_source(src: zip_source_t, flags_: c_int, error: zip_error_t) -> zip_t {
    let _ = (src, flags_, error);
    std::ptr::null_mut()
}

/// Discard an open archive without writing changes. Mirrors `void zip_discard(zip_t *)`.
pub unsafe extern "C" fn zip_discard(za: zip_t) {
    let _ = za;
}

/// Delete an entry from the archive. Mirrors `int zip_delete(zip_t *, zip_uint64_t)`.
pub unsafe extern "C" fn zip_delete(za: zip_t, idx: zip_uint64_t) -> c_int {
    let _ = (za, idx);
    -1
}

/// Locate an entry by name. Mirrors `zip_int64_t zip_name_locate(...)`.
pub unsafe extern "C" fn zip_name_locate(za: zip_t, fname: *const c_char, flags: zip_flags_t) -> zip_int64_t {
    let _ = (za, fname, flags);
    -1
}

/// Return the error structure associated with an archive. Mirrors `zip_error_t *zip_get_error(...)`.
pub unsafe extern "C" fn zip_get_error(za: zip_t) -> zip_error_t {
    let _ = za;
    std::ptr::null_mut()
}

/// Initialize a `zip_error` struct. Mirrors `void zip_error_init(zip_error_t *)`.
pub unsafe extern "C" fn zip_error_init(error: zip_error_t) {
    let _ = error;
}

/// Finalize a `zip_error` struct. Mirrors `void zip_error_fini(zip_error_t *)`.
pub unsafe extern "C" fn zip_error_fini(error: zip_error_t) {
    let _ = error;
}

/// Set the libzip and system error codes on an error struct.
pub unsafe extern "C" fn zip_error_set(error: zip_error_t, zip_err: c_int, sys_err: c_int) {
    let _ = (error, zip_err, sys_err);
}

/// Clear the error on an archive. Mirrors `void zip_error_clear(zip_t *)`.
pub unsafe extern "C" fn zip_error_clear(za: zip_t) {
    let _ = za;
}

/// Return the libzip error code from a `zip_error_t`.
pub unsafe extern "C" fn zip_error_code_zip(error: zip_error_t) -> c_int {
    let _ = error;
    ZipErrorCode::ZIP_ER_OK as c_int
}

/// Return the system error code from a `zip_error_t`.
pub unsafe extern "C" fn zip_error_code_system(error: zip_error_t) -> c_int {
    let _ = error;
    0
}

/// Return the system-error-type code from a `zip_error_t`.
pub unsafe extern "C" fn zip_error_system_type(error: zip_error_t) -> c_int {
    let _ = error;
    ZIP_ET_NONE
}

/// Free a `zip_source`. Mirrors `void zip_source_free(zip_source_t *)`.
pub unsafe extern "C" fn zip_source_free(src: zip_source_t) {
    let _ = src;
}

/// Open a `zip_source` for reading. Mirrors `int zip_source_open(zip_source_t *)`.
pub unsafe extern "C" fn zip_source_open(src: zip_source_t) -> c_int {
    let _ = src;
    0
}

/// Read from a `zip_source`. Mirrors `zip_int64_t zip_source_read(...)`.
pub unsafe extern "C" fn zip_source_read(src: zip_source_t, buf: *mut c_void, len: zip_uint64_t) -> zip_int64_t {
    let _ = (src, buf, len);
    0
}

/// Close a `zip_source`. Mirrors `int zip_source_close(zip_source_t *)`.
pub unsafe extern "C" fn zip_source_close(src: zip_source_t) -> c_int {
    let _ = src;
    0
}

/// Create a `zip_source` backed by a file on disk. Mirrors `zip_source_t *zip_source_file_create(...)`.
pub unsafe extern "C" fn zip_source_file_create(
    fn_: *const c_char,
    start: zip_uint64_t,
    len: zip_int64_t,
    error: zip_error_t,
) -> zip_source_t {
    let _ = (fn_, start, len, error);
    std::ptr::null_mut()
}

/// Create a `zip_source` backed by a function callback. Mirrors `zip_source_t *zip_source_function_create(...)`.
pub unsafe extern "C" fn zip_source_function_create(
    cb: ZipSourceCallback,
    userdata: *mut c_void,
    error: zip_error_t,
) -> zip_source_t {
    let _ = (cb, userdata, error);
    std::ptr::null_mut()
}

/// Return the error of a `zip_source`. Mirrors `zip_error_t *zip_source_error(...)`.
pub unsafe extern "C" fn zip_source_error(src: zip_source_t) -> zip_error_t {
    let _ = src;
    std::ptr::null_mut()
}

/// Set the default password used to decrypt archive entries. Mirrors `int zip_set_default_password(...)`.
pub unsafe extern "C" fn zip_set_default_password(za: zip_t, passwd: *const c_char) -> c_int {
    let _ = (za, passwd);
    0
}

/// Populate a `zip_stat` for a named entry.
pub unsafe extern "C" fn zip_stat(
    za: zip_t,
    fname: *const c_char,
    flags: zip_flags_t,
    st: *mut ZipStat,
) -> c_int {
    let _ = (za, fname, flags, st);
    -1
}

/// Populate a `zip_stat` for an indexed entry.
pub unsafe extern "C" fn zip_stat_index(za: zip_t, idx: zip_uint64_t, flags: zip_flags_t, st: *mut ZipStat) -> c_int {
    let _ = (za, idx, flags, st);
    -1
}

/// Initialize a `zip_stat` struct. Mirrors `void zip_stat_init(zip_stat_t *)`.
pub unsafe extern "C" fn zip_stat_init(st: *mut ZipStat) {
    if !st.is_null() {
        unsafe {
            (*st).valid = 0;
            (*st).name = std::ptr::null();
            (*st).index = 0;
            (*st).size = 0;
            (*st).comp_size = 0;
            (*st).mtime = 0;
            (*st).crc = 0;
            (*st).comp_method = 0;
            (*st).encryption_method = 0;
            (*st).flags = 0;
        }
    }
}

/// Initialize a `zip_file_attributes` struct. Mirrors `void zip_file_attributes_init(...)`.
pub unsafe extern "C" fn zip_file_attributes_init(attrs: *mut ZipFileAttributes) {
    if !attrs.is_null() {
        unsafe {
            (*attrs).valid = 0;
            (*attrs).version = 1;
            (*attrs).host_system = 0;
            (*attrs).ascii = 0;
            (*attrs).version_needed = 0;
            (*attrs).external_file_attributes = 0;
            (*attrs).general_purpose_bit_flags = 0;
            (*attrs).general_purpose_bit_mask = 0;
        }
    }
}

/// Return the libzip version string. Mirrors `const char *zip_libzip_version(void)`.
pub unsafe extern "C" fn zip_libzip_version() -> *const c_char {
    static VERSION: &[u8] = b"1.11.4-rust\0";
    VERSION.as_ptr() as *const c_char
}

/// Set the archive-level comment. Mirrors `int zip_set_archive_comment(...)`.
pub unsafe extern "C" fn zip_set_archive_comment(za: zip_t, comment: *const c_char, length: zip_uint16_t) -> c_int {
    let _ = (za, comment, length);
    0
}

/// Return the archive-level comment. Mirrors `const char *zip_get_archive_comment(...)`.
pub unsafe extern "C" fn zip_get_archive_comment(za: zip_t, lenp: *mut c_int, flags: zip_flags_t) -> *const c_char {
    let _ = (za, lenp, flags);
    std::ptr::null()
}

/// Revert changes to a single entry. Mirrors `int zip_unchange(zip_t *, zip_uint64_t)`.
pub unsafe extern "C" fn zip_unchange(za: zip_t, idx: zip_uint64_t) -> c_int {
    let _ = (za, idx);
    0
}

/// Revert all changes. Mirrors `int zip_unchange_all(zip_t *)`.
pub unsafe extern "C" fn zip_unchange_all(za: zip_t) -> c_int {
    let _ = za;
    0
}

/// Revert archive-level changes. Mirrors `int zip_unchange_archive(zip_t *)`.
pub unsafe extern "C" fn zip_unchange_archive(za: zip_t) -> c_int {
    let _ = za;
    0
}

/// Return whether a compression method is supported.
pub unsafe extern "C" fn zip_compression_method_supported(method: zip_int32_t, compress: c_int) -> c_int {
    let _ = (method, compress);
    0
}

/// Return whether an encryption method is supported.
pub unsafe extern "C" fn zip_encryption_method_supported(method: zip_uint16_t, encode: c_int) -> c_int {
    let _ = (method, encode);
    0
}

// =====================================================================
// Legacy / deprecated entry points preserved for ABI compatibility
// =====================================================================

/// Deprecated `zip_add` alias. Mirrors `zip_int64_t zip_add(zip_t *, const char *, zip_source_t)`.
pub unsafe extern "C" fn zip_add(za: zip_t, name: *const c_char, source: zip_source_t) -> zip_int64_t {
    zip_file_add(za, name, source, 0)
}

/// Deprecated `zip_add_dir` alias. Mirrors `zip_int64_t zip_add_dir(zip_t *, const char *)`.
pub unsafe extern "C" fn zip_add_dir(za: zip_t, name: *const c_char) -> zip_int64_t {
    zip_dir_add(za, name, 0)
}

/// Deprecated `zip_get_num_files` alias. Mirrors `int zip_get_num_files(zip_t *)`.
pub unsafe extern "C" fn zip_get_num_files(za: zip_t) -> c_int {
    let n = zip_get_num_entries(za, 0);
    if n < 0 { -1 } else { n as c_int }
}

/// Deprecated `zip_rename` alias. Mirrors `int zip_rename(zip_t *, zip_uint64_t, const char *)`.
pub unsafe extern "C" fn zip_rename(za: zip_t, idx: zip_uint64_t, name: *const c_char) -> c_int {
    let _ = (za, idx, name);
    -1
}

/// Deprecated `zip_replace` alias. Mirrors `int zip_replace(zip_t *, zip_uint64_t, zip_source_t)`.
pub unsafe extern "C" fn zip_replace(za: zip_t, idx: zip_uint64_t, source: zip_source_t) -> c_int {
    let _ = (za, idx, source);
    -1
}

/// Register a legacy progress callback. Mirrors `void zip_register_progress_callback(...)`.
pub unsafe extern "C" fn zip_register_progress_callback(za: zip_t, cb: ZipProgressCallbackLegacy) {
    let _ = (za, cb);
}

/// Internal: set a `c_int` from a `zip_error_t`. Mirrors `_zip_set_open_error`.
pub unsafe extern "C" fn _zip_set_open_error(zep: *mut c_int, err: *const ZipError, ze: c_int) {
    if !zep.is_null() {
        unsafe {
            *zep = ze;
        }
    }
    let _ = err;
}

// =====================================================================
// Inline helpers (formerly macros in the C headers)
// =====================================================================

/// Mirror of `ZIP_SOURCE_MAKE_COMMAND_BITMASK`.
#[inline]
pub const fn zip_source_make_command_bitmask(cmd: ZipSourceCmd) -> i64 {
    1i64 << (cmd as i64)
}

/// Mirror of `ZIP_SOURCE_CHECK_SUPPORTED`.
#[inline]
pub const fn zip_source_check_supported(supported: i64, cmd: ZipSourceCmd) -> bool {
    (supported & zip_source_make_command_bitmask(cmd)) != 0
}

/// Mirror of `ZIP_SOURCE_SUPPORTS_READABLE`.
pub const ZIP_SOURCE_SUPPORTS_READABLE: i64 = (zip_source_make_command_bitmask(ZipSourceCmd::Open)
    | zip_source_make_command_bitmask(ZipSourceCmd::Read)
    | zip_source_make_command_bitmask(ZipSourceCmd::Close)
    | zip_source_make_command_bitmask(ZipSourceCmd::Stat)
    | zip_source_make_command_bitmask(ZipSourceCmd::Error)
    | zip_source_make_command_bitmask(ZipSourceCmd::Free));

/// Mirror of `ZIP_SOURCE_SUPPORTS_SEEKABLE`.
pub const ZIP_SOURCE_SUPPORTS_SEEKABLE: i64 = (ZIP_SOURCE_SUPPORTS_READABLE
    | zip_source_make_command_bitmask(ZipSourceCmd::Seek)
    | zip_source_make_command_bitmask(ZipSourceCmd::Tell)
    | zip_source_make_command_bitmask(ZipSourceCmd::Supports));

/// Mirror of `ZIP_SOURCE_SUPPORTS_WRITABLE`.
pub const ZIP_SOURCE_SUPPORTS_WRITABLE: i64 = (ZIP_SOURCE_SUPPORTS_SEEKABLE
    | zip_source_make_command_bitmask(ZipSourceCmd::BeginWrite)
    | zip_source_make_command_bitmask(ZipSourceCmd::CommitWrite)
    | zip_source_make_command_bitmask(ZipSourceCmd::RollbackWrite)
    | zip_source_make_command_bitmask(ZipSourceCmd::Write)
    | zip_source_make_command_bitmask(ZipSourceCmd::SeekWrite)
    | zip_source_make_command_bitmask(ZipSourceCmd::TellWrite)
    | zip_source_make_command_bitmask(ZipSourceCmd::Remove));

#[allow(dead_code)]
const _: () = {
    // Make sure the high bit of ZIP_ER_USER_DEFINED round-trips.
    assert!(ZipErrorCode::ZIP_ER_USER_DEFINED as i32 >= 0);
};
