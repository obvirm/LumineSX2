// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! YAML serialization/deserialization helpers.
//!
//! Originally a thin wrapper around the ryml (RapidYAML) C++ library, used for
//! parsing/serializing `GameIndex.yaml`, `RedumpDatabase.yaml`, and other data
//! shipped with PCSX2.
//!
//! In Rust we use the pure-Rust `serde_yml` crate (an actively-maintained fork
//! of `serde_yaml`, by the same author). `serde_yaml` itself is in maintenance
//! mode and only receives security fixes; `serde_yml` continues to receive
//! updates and exposes the same `from_str` / `to_string` API as a drop-in
//! replacement. The backend swap is therefore a one-line `Cargo.toml` change.
//!
//! ```toml
//! [dependencies]
//! serde = { version = "1", features = ["derive"] }
//! serde_yml = "0.0.12"
//! ```
//!
//! All public functions are generic over `T: Serialize + DeserializeOwned`,
//! so the same helpers work for any data type, no per-type glue required.

use std::fs;
use std::io;
use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;
use thiserror::Error;

/// Errors produced by this module.
#[derive(Debug, Error)]
pub enum Error {
    /// Underlying I/O failure while reading or writing a file.
    #[error("yaml I/O error: {0}")]
    Io(#[from] io::Error),

    /// YAML syntax error or schema mismatch while deserializing.
    #[error("yaml parse error: {0}")]
    Parse(#[from] serde_yml::Error),
}

/// Convenience result alias for callers of this module.
pub type Result<T> = std::result::Result<T, Error>;

/// Deserialize a YAML value of type `T` from a string slice.
///
/// # Errors
/// Returns [`Error::Parse`] if `s` is not valid YAML for `T`.
pub fn from_str<T>(s: &str) -> Result<T>
where
    T: DeserializeOwned,
{
    Ok(serde_yml::from_str(s)?)
}

/// Serialize a value of type `T` to a YAML string.
///
/// # Errors
/// Returns [`Error::Parse`] if `T`'s `Serialize` impl encounters an
/// unrepresentable value (rare in practice; most failures are panics).
pub fn to_string<T>(value: &T) -> Result<String>
where
    T: Serialize,
{
    Ok(serde_yml::to_string(value)?)
}

/// Read a file from `path` and deserialize its contents as YAML into `T`.
///
/// # Errors
/// Returns [`Error::Io`] if the file cannot be read, or [`Error::Parse`]
/// if the contents are not valid YAML for `T`.
pub fn load_from_file<T>(path: &Path) -> Result<T>
where
    T: DeserializeOwned,
{
    let contents = fs::read_to_string(path)?;
    from_str(&contents)
}

/// Serialize `value` as YAML and write it to `path`.
///
/// The file is created if it does not exist, and truncated if it does.
/// Formatting mirrors `serde_yml::to_string` (block style for collections,
/// inline `flow` style for plain scalar maps where appropriate).
///
/// # Errors
/// Returns [`Error::Io`] on filesystem failure, or [`Error::Parse`] on
/// serialization failure.
pub fn save_to_file<T>(path: &Path, value: &T) -> Result<()>
where
    T: Serialize,
{
    let serialized = to_string(value)?;
    fs::write(path, serialized)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// FFI surface
// ---------------------------------------------------------------------------
//
// Generic FFI over `Serialize`/`DeserializeOwned` isn't possible without
// exposing a type-tagged ABI, so we provide two helpers that operate on the
// YAML *text* itself. Callers parse/serialize against their own types in C++
// or Rust, and use these helpers only to round-trip text through the file
// system or the in-process ryml replacement.

use std::ffi::CStr;
use std::os::raw::c_char;

/// Read a UTF-8 file path and return its contents as a heap-allocated
/// UTF-8 byte buffer.
///
/// On success, the returned pointer points to a buffer of `*out_len` bytes
/// that the caller **must** release with [`pcsx2_yaml_free`]. On failure,
/// the function returns a null pointer.
///
/// # Safety
/// - `path` must be a valid null-terminated C string.
/// - `out_len` must be a valid, non-null pointer to a `usize`.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_yaml_load_file(
    path: *const c_char,
    out_len: *mut usize,
) -> *mut u8 {
    if path.is_null() || out_len.is_null() {
        return std::ptr::null_mut();
    }

    let c_path = match CStr::from_ptr(path).to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let bytes = match fs::read(c_path) {
        Ok(b) => b,
        Err(_) => return std::ptr::null_mut(),
    };

    let mut buf = bytes.into_boxed_slice();
    let len = buf.len();
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);

    *out_len = len;
    ptr
}

/// Free a buffer previously returned by [`pcsx2_yaml_load_file`].
///
/// Passing a null pointer is a no-op (matches the C convention used by
/// `free`).
///
/// # Safety
/// - `data` must either be null or have been allocated by
///   [`pcsx2_yaml_load_file`].
/// - `len` must match the length that was reported in `out_len`.
/// - After this call, `data` is invalid and must not be used.
#[no_mangle]
pub unsafe extern "C" fn pcsx2_yaml_free(data: *mut u8, len: usize) {
    if data.is_null() {
        return;
    }
    let slice = std::slice::from_raw_parts_mut(data, len);
    let _ = Box::from_raw(slice as *mut [u8] as *mut u8);
    // The slice was created from a boxed slice originally; reconstruct
    // the box to drop it. (The cast above is a no-op layout-wise, but the
    // allocation must be freed in the same form it was made.)
    drop(Box::from_raw(std::ptr::slice_from_raw_parts_mut(data, len) as *mut [u8]));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Entry {
        name: String,
        serial: String,
        crc: Option<String>,
    }

    #[test]
    fn roundtrip_struct() {
        let entry = Entry {
            name: "Test Game".into(),
            serial: "SLUS-12345".into(),
            crc: Some("deadbeef".into()),
        };

        let yaml = to_string(&entry).unwrap();
        let parsed: Entry = from_str(&yaml).unwrap();
        assert_eq!(parsed, entry);
    }

    #[test]
    fn parse_known_good() {
        let input = "name: Test\nserial: SCUS-97123\n";
        let entry: Entry = from_str(input).unwrap();
        assert_eq!(entry.name, "Test");
        assert_eq!(entry.serial, "SCUS-97123");
        assert!(entry.crc.is_none());
    }

    #[test]
    fn parse_error_is_reported() {
        let bad = "name: : :\n  - not a map";
        let result: std::result::Result<Entry, _> = from_str(bad);
        assert!(result.is_err());
    }
}

