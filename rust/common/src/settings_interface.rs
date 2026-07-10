//! `settings_interface` — Rust port of `common/SettingsInterface.h`.
//!
//! This module defines the abstract configuration back-end contract used
//! across PCSX2 for storing typed key/value pairs in named sections. It
//! covers the INI-style read/write API, optional default-value
//! convenience methods, and an FFI vtable so that C++ back-ends can be
//! driven from Rust (and vice-versa) through a stable C ABI.
//!
//! # Trait
//!
//! [`SettingsInterface`] is the idiomatic Rust translation of the C++
//! abstract base class. Several shape changes were made for ergonomics:
//!
//! * Getters return [`Option<T>`] instead of writing through a
//!   `T* value` out-parameter and returning a `bool`. Callers that
//!   want a fallback use `.unwrap_or(default)` at the call site, or the
//!   [`get_int_or`]/[`get_string_or`] free functions below.
//! * [`SettingsInterface::save`] returns `Result<(), Error>` so that
//!   back-ends can surface I/O or parse failures instead of writing
//!   them into a borrowed `Error*` out-parameter.
//! * [`SettingsInterface::set_string_list`] takes `&[String]` rather
//!   than `&Vec<String>` so callers don't have to allocate.
//! * The C++ `__fi` default-value overloads become free functions in
//!   the [`defaults`] module (and are re-exported at the module root
//!   for convenience). Keeping them out of the trait avoids object-
//!   safety hazards and lets downstream impls avoid re-exporting the
//!   helpers themselves.
//!
//! # FFI vtable
//!
//! The C++ `SettingsInterface` is a polymorphic abstract class whose
//! dispatch goes through a compiler-generated vtable. To preserve that
//! shape across the C ABI we expose an explicit vtable: a
//! `#[repr(C)]` [`SettingsVTable`] struct of `Option<extern "C" fn(...)>`
//! pointers plus an opaque [`SettingsHandle`] that bundles the
//! user-data pointer and a `&'static` reference to the vtable. Rust
//! code that wants to talk to a foreign implementation wraps the
//! handle in [`ExternalSettingsInterface`], which derefs the function
//! pointers and implements [`SettingsInterface`] on top of them.
//!
//! This is intentionally a "manual" vtable rather than `dyn` trait
//! objects: trait objects are not `repr(C)`, so cbindgen cannot
//! describe them for the C++ side. The struct-of-function-pointers
//! pattern is layout-stable, has a predictable ABI on every platform
//! we target, and matches the way the C++ compiler itself lays out
//! a polymorphic class.
//!
//! Strings passed across the boundary use `*const c_char` (C strings
//! owned by the caller) on input and `*mut c_char` (heap-allocated,
//! ownership transferred to the receiver) on output. The
//! `get_string`-style slots return `*mut c_char`; Rust copies the
//! bytes into an owned [`String`] and frees the C allocation through
//! [`libc::free`]. The C++ side therefore should hand back buffers
//! allocated with the platform's `malloc`/`free` pair so the round
//! trip is sound on every supported target.
//!
//! The vtable itself is **deferred**: nothing in this crate currently
//! needs to call back into C++, and the C++ side is not yet calling
//! into Rust either. The types are wired up and `unsafe`-sound, but
//! they are not referenced from any `#[no_mangle] pub extern "C"`
//! entry point. Wiring them in is a follow-up step that does not
//! require further changes to the trait shape.

#![allow(clippy::missing_safety_doc)]

use std::error::Error as StdError;
use std::ffi::{c_char, c_void, CString};
use std::ptr;

/// Boxed error returned from fallible [`SettingsInterface`] methods.
///
/// Uses the boxed-trait-object shape so back-ends can return any
/// concrete error type without having to thread a single enum
/// through every layer.
pub type Error = Box<dyn StdError + Send + Sync + 'static>;

// ---------------------------------------------------------------------------
//  Trait
// ---------------------------------------------------------------------------

/// Abstract configuration back-end.
///
/// Mirrors the C++ `SettingsInterface` abstract base class. All
/// in-memory back-ends, INI files, layered files, emu-folder files,
/// command-line overlays, and Qt-side `QSettings` adapters implement
/// this trait so callers can manipulate configuration without
/// knowing which store is live.
pub trait SettingsInterface {
    /// Persist the in-memory state to the underlying storage.
    ///
    /// For back-ends without a persistent store (e.g. an in-memory
    /// layer used as a transient overlay) the natural implementation
    /// is `Ok(())`.
    fn save(&mut self) -> Result<(), Error>;

    /// Discard every value and section from the back-end.
    fn clear(&mut self);

    /// Returns `true` when the back-end has no values at all.
    fn is_empty(&self) -> bool;

    /// Look up a signed 32-bit integer. Returns `None` if the key is
    /// missing or holds a value of an incompatible type.
    fn get_int(&self, section: &str, key: &str) -> Option<i32>;

    /// Look up an unsigned 32-bit integer. Returns `None` if the key
    /// is missing or holds a value of an incompatible type.
    fn get_uint(&self, section: &str, key: &str) -> Option<u32>;

    /// Look up a 32-bit float. Returns `None` if the key is missing
    /// or holds a value of an incompatible type.
    fn get_float(&self, section: &str, key: &str) -> Option<f32>;

    /// Look up a 64-bit float. Returns `None` if the key is missing
    /// or holds a value of an incompatible type.
    fn get_double(&self, section: &str, key: &str) -> Option<f64>;

    /// Look up a boolean. Returns `None` if the key is missing or
    /// holds a value of an incompatible type.
    fn get_bool(&self, section: &str, key: &str) -> Option<bool>;

    /// Look up a string. Returns `None` if the key is missing.
    fn get_string(&self, section: &str, key: &str) -> Option<String>;

    /// Store a signed 32-bit integer, replacing any existing value.
    fn set_int(&mut self, section: &str, key: &str, value: i32);

    /// Store an unsigned 32-bit integer, replacing any existing value.
    fn set_uint(&mut self, section: &str, key: &str, value: u32);

    /// Store a 32-bit float, replacing any existing value.
    fn set_float(&mut self, section: &str, key: &str, value: f32);

    /// Store a 64-bit float, replacing any existing value.
    fn set_double(&mut self, section: &str, key: &str, value: f64);

    /// Store a boolean, replacing any existing value.
    fn set_bool(&mut self, section: &str, key: &str, value: bool);

    /// Store a string, replacing any existing value.
    fn set_string(&mut self, section: &str, key: &str, value: &str);

    /// Read an ordered list of strings. Returns an empty [`Vec`] if
    /// the key is absent.
    fn get_string_list(&self, section: &str, key: &str) -> Vec<String>;

    /// Replace an ordered list of strings. The previous value (if
    /// any) is discarded.
    fn set_string_list(&mut self, section: &str, key: &str, value: &[String]);

    /// Append `item` to the list at `(section, key)`. Creates the
    /// key (with `item` as its sole entry) if it does not exist.
    /// Returns `true` if `item` was not already present and has now
    /// been added.
    fn add_to_string_list(&mut self, section: &str, key: &str, item: &str) -> bool;

    /// Remove the first occurrence of `item` from the list at
    /// `(section, key)`. Returns `true` if an entry was removed.
    fn remove_from_string_list(&mut self, section: &str, key: &str, item: &str) -> bool;

    /// Read every key/value pair in `section`. The order of the
    /// returned vector is back-end-defined; in practice it tracks
    /// insertion order for INI files and alphabetical order for
    /// `QSettings`.
    fn get_key_value_list(&self, section: &str) -> Vec<(String, String)>;

    /// Replace every key/value pair in `section`. Pre-existing keys
    /// not present in `items` are discarded.
    fn set_key_value_list(&mut self, section: &str, items: &[(String, String)]);

    /// Returns `true` when `(section, key)` exists, regardless of
    /// the value's type.
    fn contains(&self, section: &str, key: &str) -> bool;

    /// Delete the value at `(section, key)`. Silently does nothing
    /// when the key is absent.
    fn delete_value(&mut self, section: &str, key: &str);

    /// Delete every key in `section` but leave the section header
    /// intact. Silently does nothing if the section is absent.
    fn clear_section(&mut self, section: &str);

    /// Delete the named section and all of its keys. Silently does
    /// nothing if the section is absent.
    fn remove_section(&mut self, section: &str);

    /// Remove every section that contains zero keys. Useful as a
    /// post-write cleanup so a freshly saved INI doesn't accumulate
    /// empty headers.
    fn remove_empty_sections(&mut self);
}

// ---------------------------------------------------------------------------
//  Default-value convenience methods
//
//  In C++ these are `__fi` overloads on the abstract base. Rust traits
//  can't carry default parameters, so they live as free functions in the
//  `defaults` submodule. The `SI` blanket parameter makes them callable
//  through any concrete or trait-object back-end.
// ---------------------------------------------------------------------------

/// Return the value at `(section, key)` or `default` if the key is
/// missing or holds an incompatible type.
#[inline]
pub fn get_int_or<S: SettingsInterface + ?Sized>(si: &S, section: &str, key: &str, default: i32) -> i32 {
    si.get_int(section, key).unwrap_or(default)
}

/// Return the value at `(section, key)` or `default` if the key is
/// missing or holds an incompatible type.
#[inline]
pub fn get_uint_or<S: SettingsInterface + ?Sized>(si: &S, section: &str, key: &str, default: u32) -> u32 {
    si.get_uint(section, key).unwrap_or(default)
}

/// Return the value at `(section, key)` or `default` if the key is
/// missing or holds an incompatible type.
#[inline]
pub fn get_float_or<S: SettingsInterface + ?Sized>(si: &S, section: &str, key: &str, default: f32) -> f32 {
    si.get_float(section, key).unwrap_or(default)
}

/// Return the value at `(section, key)` or `default` if the key is
/// missing or holds an incompatible type.
#[inline]
pub fn get_double_or<S: SettingsInterface + ?Sized>(si: &S, section: &str, key: &str, default: f64) -> f64 {
    si.get_double(section, key).unwrap_or(default)
}

/// Return the value at `(section, key)` or `default` if the key is
/// missing or holds an incompatible type.
#[inline]
pub fn get_bool_or<S: SettingsInterface + ?Sized>(si: &S, section: &str, key: &str, default: bool) -> bool {
    si.get_bool(section, key).unwrap_or(default)
}

/// Return the value at `(section, key)` or `default` if the key is
/// missing.
#[inline]
pub fn get_string_or<S: SettingsInterface + ?Sized>(si: &S, section: &str, key: &str, default: &str) -> String {
    si.get_string(section, key).unwrap_or_else(|| default.to_string())
}

/// Optional-valued getters. `Ok(Some(v))` means "key present and
/// well-typed", `Ok(None)` means "key absent", and `Err(_)` is
/// reserved for future use (currently always `Ok`).
pub mod defaults {
    use super::{Error, SettingsInterface};

    /// Look up a signed integer wrapped in [`Option`].
    pub fn get_optional_int<S: SettingsInterface + ?Sized>(
        si: &S,
        section: &str,
        key: &str,
    ) -> Result<Option<i32>, Error> {
        Ok(si.get_int(section, key))
    }

    /// Look up an unsigned integer wrapped in [`Option`].
    pub fn get_optional_uint<S: SettingsInterface + ?Sized>(
        si: &S,
        section: &str,
        key: &str,
    ) -> Result<Option<u32>, Error> {
        Ok(si.get_uint(section, key))
    }

    /// Look up a 32-bit float wrapped in [`Option`].
    pub fn get_optional_float<S: SettingsInterface + ?Sized>(
        si: &S,
        section: &str,
        key: &str,
    ) -> Result<Option<f32>, Error> {
        Ok(si.get_float(section, key))
    }

    /// Look up a 64-bit float wrapped in [`Option`].
    pub fn get_optional_double<S: SettingsInterface + ?Sized>(
        si: &S,
        section: &str,
        key: &str,
    ) -> Result<Option<f64>, Error> {
        Ok(si.get_double(section, key))
    }

    /// Look up a boolean wrapped in [`Option`].
    pub fn get_optional_bool<S: SettingsInterface + ?Sized>(
        si: &S,
        section: &str,
        key: &str,
    ) -> Result<Option<bool>, Error> {
        Ok(si.get_bool(section, key))
    }

    /// Look up a string wrapped in [`Option`].
    pub fn get_optional_string<S: SettingsInterface + ?Sized>(
        si: &S,
        section: &str,
        key: &str,
    ) -> Result<Option<String>, Error> {
        Ok(si.get_string(section, key))
    }

    /// Set a signed integer, or delete the key if `value` is [`None`].
    pub fn set_optional_int<S: SettingsInterface + ?Sized>(
        si: &mut S,
        section: &str,
        key: &str,
        value: Option<i32>,
    ) {
        match value {
            Some(v) => si.set_int(section, key, v),
            None => si.delete_value(section, key),
        }
    }

    /// Set an unsigned integer, or delete the key if `value` is
    /// [`None`].
    pub fn set_optional_uint<S: SettingsInterface + ?Sized>(
        si: &mut S,
        section: &str,
        key: &str,
        value: Option<u32>,
    ) {
        match value {
            Some(v) => si.set_uint(section, key, v),
            None => si.delete_value(section, key),
        }
    }

    /// Set a 32-bit float, or delete the key if `value` is [`None`].
    pub fn set_optional_float<S: SettingsInterface + ?Sized>(
        si: &mut S,
        section: &str,
        key: &str,
        value: Option<f32>,
    ) {
        match value {
            Some(v) => si.set_float(section, key, v),
            None => si.delete_value(section, key),
        }
    }

    /// Set a 64-bit float, or delete the key if `value` is [`None`].
    pub fn set_optional_double<S: SettingsInterface + ?Sized>(
        si: &mut S,
        section: &str,
        key: &str,
        value: Option<f64>,
    ) {
        match value {
            Some(v) => si.set_double(section, key, v),
            None => si.delete_value(section, key),
        }
    }

    /// Set a boolean, or delete the key if `value` is [`None`].
    pub fn set_optional_bool<S: SettingsInterface + ?Sized>(
        si: &mut S,
        section: &str,
        key: &str,
        value: Option<bool>,
    ) {
        match value {
            Some(v) => si.set_bool(section, key, v),
            None => si.delete_value(section, key),
        }
    }

    /// Set a string, or delete the key if `value` is [`None`].
    pub fn set_optional_string<S: SettingsInterface + ?Sized>(
        si: &mut S,
        section: &str,
        key: &str,
        value: Option<&str>,
    ) {
        match value {
            Some(v) => si.set_string(section, key, v),
            None => si.delete_value(section, key),
        }
    }

    /// Copy a signed integer from `src` to `dst`, deleting it on the
    /// destination when it is absent on the source.
    pub fn copy_int_value<S: SettingsInterface + ?Sized>(
        dst: &mut S,
        src: &S,
        section: &str,
        key: &str,
    ) {
        match src.get_int(section, key) {
            Some(v) => dst.set_int(section, key, v),
            None => dst.delete_value(section, key),
        }
    }

    /// Copy an unsigned integer.
    pub fn copy_uint_value<S: SettingsInterface + ?Sized>(
        dst: &mut S,
        src: &S,
        section: &str,
        key: &str,
    ) {
        match src.get_uint(section, key) {
            Some(v) => dst.set_uint(section, key, v),
            None => dst.delete_value(section, key),
        }
    }

    /// Copy a 32-bit float.
    pub fn copy_float_value<S: SettingsInterface + ?Sized>(
        dst: &mut S,
        src: &S,
        section: &str,
        key: &str,
    ) {
        match src.get_float(section, key) {
            Some(v) => dst.set_float(section, key, v),
            None => dst.delete_value(section, key),
        }
    }

    /// Copy a 64-bit float.
    pub fn copy_double_value<S: SettingsInterface + ?Sized>(
        dst: &mut S,
        src: &S,
        section: &str,
        key: &str,
    ) {
        match src.get_double(section, key) {
            Some(v) => dst.set_double(section, key, v),
            None => dst.delete_value(section, key),
        }
    }

    /// Copy a boolean.
    pub fn copy_bool_value<S: SettingsInterface + ?Sized>(
        dst: &mut S,
        src: &S,
        section: &str,
        key: &str,
    ) {
        match src.get_bool(section, key) {
            Some(v) => dst.set_bool(section, key, v),
            None => dst.delete_value(section, key),
        }
    }

    /// Copy a string.
    pub fn copy_string_value<S: SettingsInterface + ?Sized>(
        dst: &mut S,
        src: &S,
        section: &str,
        key: &str,
    ) {
        match src.get_string(section, key) {
            Some(v) => dst.set_string(section, key, &v),
            None => dst.delete_value(section, key),
        }
    }

    /// Copy a string list. Empty source lists delete the destination
    /// key, mirroring the C++ `CopyStringListValue` contract.
    pub fn copy_string_list_value<S: SettingsInterface + ?Sized>(
        dst: &mut S,
        src: &S,
        section: &str,
        key: &str,
    ) {
        let value = src.get_string_list(section, key);
        if value.is_empty() {
            dst.delete_value(section, key);
        } else {
            dst.set_string_list(section, key, &value);
        }
    }

    /// Copy every key/value pair from `src`'s `section` to `dst`'s
    /// same-named section.
    pub fn copy_keys_and_values<S: SettingsInterface + ?Sized>(
        dst: &mut S,
        src: &S,
        section: &str,
    ) {
        let items = src.get_key_value_list(section);
        dst.set_key_value_list(section, &items);
    }
}

// ---------------------------------------------------------------------------
//  FFI vtable
//
//  See the module-level documentation for the rationale. The layout is
//  frozen: any reordering would silently break ABI compatibility with
//  C++ code that has been built against an earlier version of this
//  header.
// ---------------------------------------------------------------------------

/// Opaque, C-ABI-stable handle to a foreign [`SettingsInterface`].
///
/// `user_data` is owned by the foreign side; the Rust wrapper treats
/// it as borrowed-for-the-lifetime-of-the-handle and never frees it.
/// `vtable` must point to a [`SettingsVTable`] whose function
/// pointers remain valid for at least as long as the handle itself.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SettingsHandle {
    /// Foreign `this` pointer (or whatever the back-end chooses to
    /// pass as its opaque context).
    pub user_data: *mut c_void,
    /// Pointer to the dispatch table.
    pub vtable: *const SettingsVTable,
}

/// C-ABI vtable for [`SettingsInterface`].
///
/// Each slot is an `Option<extern "C" fn(...)>` whose first parameter
/// is the opaque `user_data` pointer the foreign side stored in the
/// [`SettingsHandle`]. A `None` slot signals "method not implemented"
/// and the Rust wrapper surfaces that as `None` (for getters) or a
/// silent no-op (for setters), matching how the C++ `= 0` contract
/// is interpreted when called through a `SettingsInterface*` that
/// happens to be `nullptr`-dispatched.
///
/// String output slots must return either `ptr::null_mut()` ("no
/// value") or a heap-allocated NUL-terminated C string whose
/// ownership transfers to Rust. The wrapper uses [`libc::free`] on
/// the returned pointer, so the C++ side must allocate with the
/// platform `malloc`/`free` pair.
///
/// Vector output slots follow the same ownership transfer: the
/// caller (foreign side) returns a pointer to a contiguous array of
/// `*mut c_char` followed by a `Vec`-shaped layout; the wrapper
/// copies each entry into a [`String`], frees the C-string entries,
/// and frees the outer array.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SettingsVTable {
    /// `save` -> bool. `false` indicates failure; the slot does not
    /// surface a structured error.
    pub save: Option<unsafe extern "C" fn(user_data: *mut c_void) -> bool>,
    /// `clear` -> ()
    pub clear: Option<unsafe extern "C" fn(user_data: *mut c_void)>,
    /// `is_empty` -> bool
    pub is_empty: Option<unsafe extern "C" fn(user_data: *const c_void) -> bool>,

    /// `get_int` -> bool. Writes the value through `value_out` on
    /// success.
    pub get_int: Option<
        unsafe extern "C" fn(
            user_data: *const c_void,
            section: *const c_char,
            key: *const c_char,
            value_out: *mut i32,
        ) -> bool,
    >,
    /// `get_uint`
    pub get_uint: Option<
        unsafe extern "C" fn(
            user_data: *const c_void,
            section: *const c_char,
            key: *const c_char,
            value_out: *mut u32,
        ) -> bool,
    >,
    /// `get_float`
    pub get_float: Option<
        unsafe extern "C" fn(
            user_data: *const c_void,
            section: *const c_char,
            key: *const c_char,
            value_out: *mut f32,
        ) -> bool,
    >,
    /// `get_double`
    pub get_double: Option<
        unsafe extern "C" fn(
            user_data: *const c_void,
            section: *const c_char,
            key: *const c_char,
            value_out: *mut f64,
        ) -> bool,
    >,
    /// `get_bool`
    pub get_bool: Option<
        unsafe extern "C" fn(
            user_data: *const c_void,
            section: *const c_char,
            key: *const c_char,
            value_out: *mut bool,
        ) -> bool,
    >,
    /// `get_string` -> `*mut c_char`. Ownership transfers to Rust.
    pub get_string: Option<
        unsafe extern "C" fn(
            user_data: *const c_void,
            section: *const c_char,
            key: *const c_char,
        ) -> *mut c_char,
    >,

    /// `set_int`
    pub set_int: Option<
        unsafe extern "C" fn(user_data: *mut c_void, section: *const c_char, key: *const c_char, value: i32),
    >,
    /// `set_uint`
    pub set_uint: Option<
        unsafe extern "C" fn(user_data: *mut c_void, section: *const c_char, key: *const c_char, value: u32),
    >,
    /// `set_float`
    pub set_float: Option<
        unsafe extern "C" fn(user_data: *mut c_void, section: *const c_char, key: *const c_char, value: f32),
    >,
    /// `set_double`
    pub set_double: Option<
        unsafe extern "C" fn(user_data: *mut c_void, section: *const c_char, key: *const c_char, value: f64),
    >,
    /// `set_bool`
    pub set_bool: Option<
        unsafe extern "C" fn(user_data: *mut c_void, section: *const c_char, key: *const c_char, value: bool),
    >,
    /// `set_string`
    pub set_string: Option<
        unsafe extern "C" fn(user_data: *mut c_void, section: *const c_char, key: *const c_char, value: *const c_char),
    >,

    /// `get_string_list` -> pair of `(items_ptr, len)` packed in an
    /// out-array; see [`SettingsStringList`] for the wire shape.
    pub get_string_list: Option<
        unsafe extern "C" fn(
            user_data: *const c_void,
            section: *const c_char,
            key: *const c_char,
            out: *mut SettingsStringList,
        ),
    >,
    /// `set_string_list`
    pub set_string_list: Option<
        unsafe extern "C" fn(
            user_data: *mut c_void,
            section: *const c_char,
            key: *const c_char,
            items: *const *const c_char,
            len: usize,
        ),
    >,
    /// `add_to_string_list`
    pub add_to_string_list: Option<
        unsafe extern "C" fn(
            user_data: *mut c_void,
            section: *const c_char,
            key: *const c_char,
            item: *const c_char,
        ) -> bool,
    >,
    /// `remove_from_string_list`
    pub remove_from_string_list: Option<
        unsafe extern "C" fn(
            user_data: *mut c_void,
            section: *const c_char,
            key: *const c_char,
            item: *const c_char,
        ) -> bool,
    >,

    /// `get_key_value_list` -> pair of `(pairs_ptr, len)` packed in
    /// an out-array; see [`SettingsKeyValueList`] for the wire
    /// shape.
    pub get_key_value_list: Option<
        unsafe extern "C" fn(
            user_data: *const c_void,
            section: *const c_char,
            out: *mut SettingsKeyValueList,
        ),
    >,
    /// `set_key_value_list`
    pub set_key_value_list: Option<
        unsafe extern "C" fn(
            user_data: *mut c_void,
            section: *const c_char,
            items: *const SettingsKeyValue,
            len: usize,
        ),
    >,

    /// `contains`
    pub contains: Option<
        unsafe extern "C" fn(user_data: *const c_void, section: *const c_char, key: *const c_char) -> bool,
    >,
    /// `delete_value`
    pub delete_value: Option<
        unsafe extern "C" fn(user_data: *mut c_void, section: *const c_char, key: *const c_char),
    >,
    /// `clear_section`
    pub clear_section: Option<unsafe extern "C" fn(user_data: *mut c_void, section: *const c_char)>,
    /// `remove_section`
    pub remove_section: Option<unsafe extern "C" fn(user_data: *mut c_void, section: *const c_char)>,
    /// `remove_empty_sections`
    pub remove_empty_sections: Option<unsafe extern "C" fn(user_data: *mut c_void)>,
}

/// Wire shape for a string-list crossing the FFI boundary.
///
/// The C++ side populates `items` (a heap-allocated array of
/// `*mut c_char`, NUL-terminated) and `len`, transferring ownership
/// of both the array and each individual C string to Rust.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SettingsStringList {
    /// Pointer to a heap-allocated array of C strings.
    pub items: *mut *mut c_char,
    /// Number of entries in `items`.
    pub len: usize,
}

/// Wire shape for a single key/value pair.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SettingsKeyValue {
    /// Key. Heap-allocated C string, ownership transfers to Rust.
    pub key: *mut c_char,
    /// Value. Heap-allocated C string, ownership transfers to Rust.
    pub value: *mut c_char,
}

/// Wire shape for a key/value list crossing the FFI boundary.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SettingsKeyValueList {
    /// Pointer to a heap-allocated array of [`SettingsKeyValue`].
    pub items: *mut SettingsKeyValue,
    /// Number of entries in `items`.
    pub len: usize,
}

// ---------------------------------------------------------------------------
//  Wrapper
//
//  `ExternalSettingsInterface` adapts a `SettingsHandle` to the
//  idiomatic [`SettingsInterface`] trait. Each trait method either
//  derefs the matching vtable slot (translating between C strings /
//  raw pointers and Rust slices / `String`s) or returns a sensible
//  fallback when the slot is `None`.
// ---------------------------------------------------------------------------

/// Rust-facing wrapper around a foreign [`SettingsHandle`].
///
/// Constructed from a raw handle with [`ExternalSettingsInterface::from_handle`].
/// The wrapper does not take ownership of the foreign state: the
/// caller is responsible for ensuring the handle (and the
/// [`SettingsVTable`] it points to) outlive the wrapper.
///
/// `ExternalSettingsInterface` is `!Send` and `!Sync` because the
/// underlying C++ object may be running on a foreign thread with its
/// own synchronisation rules; the trait does not promise thread
/// safety, and Rust callers that want cross-thread access should
/// wrap it in the same synchronisation primitive the C++ side uses.
pub struct ExternalSettingsInterface {
    handle: SettingsHandle,
}

impl ExternalSettingsInterface {
    /// Wrap a raw C handle. The handle must remain valid for the
    /// lifetime of the returned wrapper.
    ///
    /// # Safety
    ///
    /// * `handle.user_data` must be a valid opaque pointer the
    ///   foreign back-end will accept for every vtable slot.
    /// * `handle.vtable` must point to a [`SettingsVTable`] whose
    ///   non-`None` slots are valid `extern "C"` functions with the
    ///   signatures documented on [`SettingsVTable`].
    pub const unsafe fn from_handle(handle: SettingsHandle) -> Self {
        Self { handle }
    }

    /// Borrow the underlying raw handle. Useful for code that needs
    /// to forward it back across the FFI boundary.
    pub const fn handle(&self) -> SettingsHandle {
        self.handle
    }

    /// Look up a vtable slot. Returns `None` if either the vtable
    /// pointer or the requested slot is null.
    #[inline]
    fn slot<F>(&self, f: impl FnOnce(&SettingsVTable) -> Option<F>) -> Option<F> {
        if self.handle.vtable.is_null() {
            return None;
        }
        // SAFETY: `from_handle` requires a valid vtable pointer; we
        // never write through it.
        let vtable = unsafe { &*self.handle.vtable };
        f(vtable)
    }
}

// ---- C string helpers ------------------------------------------------------

/// Borrow a `*const c_char` as a `&str`, returning `None` on null or
/// invalid UTF-8.
#[inline]
fn cstr_to_str<'a>(ptr: *const c_char) -> Option<&'a str> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: caller guarantees the pointer is a valid C string.
    unsafe { CStr::from_ptr(ptr) }.to_str().ok()
}

/// Take ownership of a `*mut c_char` produced by the C++ side and
/// copy it into a Rust [`String`]. The original allocation is freed
/// via [`libc::free`].
#[inline]
unsafe fn take_c_string(ptr: *mut c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    // SAFETY: caller (the C++ side) guarantees this is a valid
    // `malloc`-allocated, NUL-terminated C string.
    let s = CStr::from_ptr(ptr).to_string_lossy().into_owned();
    libc_free(ptr as *mut core::ffi::c_void);
    Some(s)
}

/// Free a buffer the C++ side gave us through one of the list
/// output slots.
///
/// # Safety
///
/// `items` must have been allocated by the platform's `malloc` (or a
/// compatible allocator) and `len` must equal the number of
/// elements it was originally allocated with. Each element must be
/// either a valid heap-allocated C string or a null pointer.
unsafe fn free_string_list(items: *mut *mut c_char, len: usize) {
    if items.is_null() || len == 0 {
        return;
    }
    for i in 0..len {
        // SAFETY: caller-bounded indices.
        let elem_ptr = items.add(i);
        // SAFETY: the C++ side guarantees each element is either
        // null or a malloc'd C string.
        let cstr = ptr::read(elem_ptr);
        if !cstr.is_null() {
            libc_free(cstr as *mut core::ffi::c_void);
        }
    }
    // SAFETY: caller guarantees the outer buffer was malloc'd.
    libc_free(items.cast::<core::ffi::c_void>());
}

/// Free a buffer the C++ side gave us through one of the
/// key/value-list output slots.
unsafe fn free_key_value_list(items: *mut SettingsKeyValue, len: usize) {
    if items.is_null() || len == 0 {
        return;
    }
    for i in 0..len {
        let kv_ptr = items.add(i);
        // SAFETY: the C++ side guarantees the array contents are
        // well-initialised.
        let kv = ptr::read(kv_ptr);
        if !kv.key.is_null() {
            libc_free(kv.key as *mut core::ffi::c_void);
        }
        if !kv.value.is_null() {
            libc_free(kv.value as *mut core::ffi::c_void);
        }
    }
    // SAFETY: caller guarantees the outer buffer was malloc'd.
    libc_free(items.cast::<core::ffi::c_void>());
}

/// Wrapper around [`libc::free`] that is `no_mangle`-free and works
/// even when the `libc` crate is unavailable on a target platform.
#[inline]
unsafe fn libc_free(ptr: *mut c_void) {
    // `libc` is a thin wrapper around the platform libc. When the
    // crate is not in scope (this file does not import it), fall
    // back to the platform-specific symbol directly.
    extern "C" {
        fn free(ptr: *mut c_void);
    }
    free(ptr);
}

use std::ffi::CStr;

// ---------------------------------------------------------------------------
//  Trait impl for ExternalSettingsInterface
// ---------------------------------------------------------------------------

impl SettingsInterface for ExternalSettingsInterface {
    fn save(&mut self) -> Result<(), Error> {
        let user_data = self.handle.user_data;
        match self.slot(|v| v.save) {
            Some(f) => {
                // SAFETY: the contract on `save` says the vtable
                // slot is a valid extern "C" function pointer whose
                // first argument is the same user_data the handle
                // carries.
                if unsafe { f(user_data) } {
                    Ok(())
                } else {
                    Err("settings: foreign save returned failure".into())
                }
            }
            None => Ok(()),
        }
    }

    fn clear(&mut self) {
        let user_data = self.handle.user_data;
        if let Some(f) = self.slot(|v| v.clear) {
            // SAFETY: see `save`.
            unsafe { f(user_data) };
        }
    }

    fn is_empty(&self) -> bool {
        let user_data = self.handle.user_data.cast_const();
        match self.slot(|v| v.is_empty) {
            Some(f) => unsafe { f(user_data) },
            None => true,
        }
    }

    fn get_int(&self, section: &str, key: &str) -> Option<i32> {
        let user_data = self.handle.user_data.cast_const();
        let section = CString::new(section).ok()?;
        let key = CString::new(key).ok()?;
        let f = self.slot(|v| v.get_int)?;
        let mut out = 0i32;
        // SAFETY: section and key outlive the call, `out` is a
        // stack-local valid for writes.
        unsafe { f(user_data, section.as_ptr(), key.as_ptr(), &mut out) }.then_some(out)
    }

    fn get_uint(&self, section: &str, key: &str) -> Option<u32> {
        let user_data = self.handle.user_data.cast_const();
        let section = CString::new(section).ok()?;
        let key = CString::new(key).ok()?;
        let f = self.slot(|v| v.get_uint)?;
        let mut out = 0u32;
        unsafe { f(user_data, section.as_ptr(), key.as_ptr(), &mut out) }.then_some(out)
    }

    fn get_float(&self, section: &str, key: &str) -> Option<f32> {
        let user_data = self.handle.user_data.cast_const();
        let section = CString::new(section).ok()?;
        let key = CString::new(key).ok()?;
        let f = self.slot(|v| v.get_float)?;
        let mut out = 0f32;
        unsafe { f(user_data, section.as_ptr(), key.as_ptr(), &mut out) }.then_some(out)
    }

    fn get_double(&self, section: &str, key: &str) -> Option<f64> {
        let user_data = self.handle.user_data.cast_const();
        let section = CString::new(section).ok()?;
        let key = CString::new(key).ok()?;
        let f = self.slot(|v| v.get_double)?;
        let mut out = 0f64;
        unsafe { f(user_data, section.as_ptr(), key.as_ptr(), &mut out) }.then_some(out)
    }

    fn get_bool(&self, section: &str, key: &str) -> Option<bool> {
        let user_data = self.handle.user_data.cast_const();
        let section = CString::new(section).ok()?;
        let key = CString::new(key).ok()?;
        let f = self.slot(|v| v.get_bool)?;
        let mut out = false;
        unsafe { f(user_data, section.as_ptr(), key.as_ptr(), &mut out) }.then_some(out)
    }

    fn get_string(&self, section: &str, key: &str) -> Option<String> {
        let user_data = self.handle.user_data.cast_const();
        let section = CString::new(section).ok()?;
        let key = CString::new(key).ok()?;
        let f = self.slot(|v| v.get_string)?;
        // SAFETY: ownership transfers to Rust on success.
        unsafe { take_c_string(f(user_data, section.as_ptr(), key.as_ptr())) }
    }

    fn set_int(&mut self, section: &str, key: &str, value: i32) {
        let user_data = self.handle.user_data;
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return,
        };
        let key = match CString::new(key) {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Some(f) = self.slot(|v| v.set_int) {
            // SAFETY: see `save`.
            unsafe { f(user_data, section.as_ptr(), key.as_ptr(), value) };
        }
    }

    fn set_uint(&mut self, section: &str, key: &str, value: u32) {
        let user_data = self.handle.user_data;
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return,
        };
        let key = match CString::new(key) {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Some(f) = self.slot(|v| v.set_uint) {
            unsafe { f(user_data, section.as_ptr(), key.as_ptr(), value) };
        }
    }

    fn set_float(&mut self, section: &str, key: &str, value: f32) {
        let user_data = self.handle.user_data;
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return,
        };
        let key = match CString::new(key) {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Some(f) = self.slot(|v| v.set_float) {
            unsafe { f(user_data, section.as_ptr(), key.as_ptr(), value) };
        }
    }

    fn set_double(&mut self, section: &str, key: &str, value: f64) {
        let user_data = self.handle.user_data;
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return,
        };
        let key = match CString::new(key) {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Some(f) = self.slot(|v| v.set_double) {
            unsafe { f(user_data, section.as_ptr(), key.as_ptr(), value) };
        }
    }

    fn set_bool(&mut self, section: &str, key: &str, value: bool) {
        let user_data = self.handle.user_data;
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return,
        };
        let key = match CString::new(key) {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Some(f) = self.slot(|v| v.set_bool) {
            unsafe { f(user_data, section.as_ptr(), key.as_ptr(), value) };
        }
    }

    fn set_string(&mut self, section: &str, key: &str, value: &str) {
        let user_data = self.handle.user_data;
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return,
        };
        let key = match CString::new(key) {
            Ok(s) => s,
            Err(_) => return,
        };
        let value = match CString::new(value) {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Some(f) = self.slot(|v| v.set_string) {
            unsafe { f(user_data, section.as_ptr(), key.as_ptr(), value.as_ptr()) };
        }
    }

    fn get_string_list(&self, section: &str, key: &str) -> Vec<String> {
        let user_data = self.handle.user_data.cast_const();
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let key = match CString::new(key) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let f = match self.slot(|v| v.get_string_list) {
            Some(f) => f,
            None => return Vec::new(),
        };
        let mut out = SettingsStringList {
            items: ptr::null_mut(),
            len: 0,
        };
        // SAFETY: out is a stack-local valid for writes.
        unsafe { f(user_data, section.as_ptr(), key.as_ptr(), &mut out) };
        if out.items.is_null() || out.len == 0 {
            return Vec::new();
        }
        let mut result = Vec::with_capacity(out.len);
        for i in 0..out.len {
            // SAFETY: caller-bounded index.
            let cstr = unsafe { ptr::read(out.items.add(i)) };
            // SAFETY: each element is either null or a malloc'd
            // C string.
            let owned = unsafe { take_c_string(cstr) };
            if let Some(s) = owned {
                result.push(s);
            }
        }
        // SAFETY: caller allocated the outer buffer.
        unsafe { libc_free(out.items.cast()) };
        result
    }

    fn set_string_list(&mut self, section: &str, key: &str, value: &[String]) {
        let user_data = self.handle.user_data;
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return,
        };
        let key = match CString::new(key) {
            Ok(s) => s,
            Err(_) => return,
        };
        let f = match self.slot(|v| v.set_string_list) {
            Some(f) => f,
            None => return,
        };
        // Marshal the `&[String]` into a C-friendly layout: a
        // heap-allocated array of `*const c_char` pointing at
        // NUL-terminated copies of each Rust string. The array
        // owns the CString allocations and is freed after the call.
        let mut cstrings: Vec<CString> = Vec::with_capacity(value.len());
        for s in value {
            if let Ok(c) = CString::new(s.as_str()) {
                cstrings.push(c);
            }
        }
        let ptrs: Vec<*const c_char> = cstrings.iter().map(|c| c.as_ptr()).collect();
        // SAFETY: ptrs.as_ptr() is valid for `ptrs.len()` reads;
        // cstrings keeps the underlying memory alive for the call.
        unsafe { f(user_data, section.as_ptr(), key.as_ptr(), ptrs.as_ptr(), ptrs.len()) };
        // ptrs is dropped here; cstrings is dropped after, freeing
        // the CString buffers.
    }

    fn add_to_string_list(&mut self, section: &str, key: &str, item: &str) -> bool {
        let user_data = self.handle.user_data;
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let key = match CString::new(key) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let item = match CString::new(item) {
            Ok(s) => s,
            Err(_) => return false,
        };
        match self.slot(|v| v.add_to_string_list) {
            Some(f) => unsafe { f(user_data, section.as_ptr(), key.as_ptr(), item.as_ptr()) },
            None => false,
        }
    }

    fn remove_from_string_list(&mut self, section: &str, key: &str, item: &str) -> bool {
        let user_data = self.handle.user_data;
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let key = match CString::new(key) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let item = match CString::new(item) {
            Ok(s) => s,
            Err(_) => return false,
        };
        match self.slot(|v| v.remove_from_string_list) {
            Some(f) => unsafe { f(user_data, section.as_ptr(), key.as_ptr(), item.as_ptr()) },
            None => false,
        }
    }

    fn get_key_value_list(&self, section: &str) -> Vec<(String, String)> {
        let user_data = self.handle.user_data.cast_const();
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let f = match self.slot(|v| v.get_key_value_list) {
            Some(f) => f,
            None => return Vec::new(),
        };
        let mut out = SettingsKeyValueList {
            items: ptr::null_mut(),
            len: 0,
        };
        // SAFETY: out is a stack-local valid for writes.
        unsafe { f(user_data, section.as_ptr(), &mut out) };
        if out.items.is_null() || out.len == 0 {
            return Vec::new();
        }
        let mut result = Vec::with_capacity(out.len);
        for i in 0..out.len {
            // SAFETY: caller-bounded index.
            let kv = unsafe { ptr::read(out.items.add(i)) };
            // SAFETY: each pair is either null or a malloc'd
            // C string.
            let key = unsafe { take_c_string(kv.key) }.unwrap_or_default();
            let value = unsafe { take_c_string(kv.value) }.unwrap_or_default();
            result.push((key, value));
        }
        // SAFETY: caller allocated the outer buffer.
        unsafe { libc_free(out.items.cast()) };
        result
    }

    fn set_key_value_list(&mut self, section: &str, items: &[(String, String)]) {
        let user_data = self.handle.user_data;
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return,
        };
        let f = match self.slot(|v| v.set_key_value_list) {
            Some(f) => f,
            None => return,
        };
        // Marshal the `&[(String, String)]` into a heap-allocated
        // array of `SettingsKeyValue`. Each entry holds two CString
        // pointers that remain valid for the duration of the call.
        let mut cstrings: Vec<CString> = Vec::with_capacity(items.len() * 2);
        let mut kv_array: Vec<SettingsKeyValue> = Vec::with_capacity(items.len());
        for (k, v) in items {
            let Ok(kc) = CString::new(k.as_str()) else { continue };
            let Ok(vc) = CString::new(v.as_str()) else { continue };
            let kp = kc.as_ptr();
            let vp = vc.as_ptr();
            cstrings.push(kc);
            cstrings.push(vc);
            kv_array.push(SettingsKeyValue { key: kp as *mut _, value: vp as *mut _ });
        }
        // SAFETY: kv_array.as_ptr() is valid for `kv_array.len()`
        // reads; cstrings keeps the underlying memory alive.
        unsafe { f(user_data, section.as_ptr(), kv_array.as_ptr(), kv_array.len()) };
        // kv_array is dropped here; cstrings is dropped after,
        // freeing the CString buffers.
    }

    fn contains(&self, section: &str, key: &str) -> bool {
        let user_data = self.handle.user_data.cast_const();
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let key = match CString::new(key) {
            Ok(s) => s,
            Err(_) => return false,
        };
        match self.slot(|v| v.contains) {
            Some(f) => unsafe { f(user_data, section.as_ptr(), key.as_ptr()) },
            None => false,
        }
    }

    fn delete_value(&mut self, section: &str, key: &str) {
        let user_data = self.handle.user_data;
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return,
        };
        let key = match CString::new(key) {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Some(f) = self.slot(|v| v.delete_value) {
            unsafe { f(user_data, section.as_ptr(), key.as_ptr()) };
        }
    }

    fn clear_section(&mut self, section: &str) {
        let user_data = self.handle.user_data;
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Some(f) = self.slot(|v| v.clear_section) {
            unsafe { f(user_data, section.as_ptr()) };
        }
    }

    fn remove_section(&mut self, section: &str) {
        let user_data = self.handle.user_data;
        let section = match CString::new(section) {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Some(f) = self.slot(|v| v.remove_section) {
            unsafe { f(user_data, section.as_ptr()) };
        }
    }

    fn remove_empty_sections(&mut self) {
        let user_data = self.handle.user_data;
        if let Some(f) = self.slot(|v| v.remove_empty_sections) {
            unsafe { f(user_data) };
        }
    }
}

// ---------------------------------------------------------------------------
//  Trait extension: dynamic dispatch through `&dyn SettingsInterface`
//
//  The wrapper above is for foreign-ABI back-ends. For native Rust
//  back-ends, callers can simply take a `Box<dyn SettingsInterface>`
//  or `&dyn SettingsInterface` parameter and dispatch through the
//  vtable Rust generates automatically. The convenience helpers
//  below make that ergonomic.
// ---------------------------------------------------------------------------

/// Convenience extension trait providing the same default-value
/// ergonomics as `&dyn SettingsInterface` callers had in C++.
///
/// Inherent trait methods are intentionally absent from
/// [`SettingsInterface`] so the trait remains object-safe; this
/// extension layer gives callers the unwrap-on-default syntax
/// without bloating the trait.
pub trait SettingsInterfaceExt: SettingsInterface {
    /// Read an integer or fall back to `default`.
    #[inline]
    fn get_int_or(&self, section: &str, key: &str, default: i32) -> i32 {
        get_int_or(self, section, key, default)
    }
    /// Read a `u32` or fall back to `default`.
    #[inline]
    fn get_uint_or(&self, section: &str, key: &str, default: u32) -> u32 {
        get_uint_or(self, section, key, default)
    }
    /// Read a `f32` or fall back to `default`.
    #[inline]
    fn get_float_or(&self, section: &str, key: &str, default: f32) -> f32 {
        get_float_or(self, section, key, default)
    }
    /// Read a `f64` or fall back to `default`.
    #[inline]
    fn get_double_or(&self, section: &str, key: &str, default: f64) -> f64 {
        get_double_or(self, section, key, default)
    }
    /// Read a `bool` or fall back to `default`.
    #[inline]
    fn get_bool_or(&self, section: &str, key: &str, default: bool) -> bool {
        get_bool_or(self, section, key, default)
    }
    /// Read a `String` or fall back to `default`.
    #[inline]
    fn get_string_or(&self, section: &str, key: &str, default: &str) -> String {
        get_string_or(self, section, key, default)
    }
}

/// Blanket impl so every [`SettingsInterface`] automatically picks
/// up the extension methods.
impl<T: SettingsInterface + ?Sized> SettingsInterfaceExt for T {}