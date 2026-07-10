// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Rust reimplementation of PCSX2's `common/MemorySettingsInterface.{h,cpp}`.
//!
//! `MemorySettingsInterface` is an in-process implementation of the
//! [`SettingsInterface`] trait. PCSX2 uses it in `pcsx2-gsrunner` and
//! other places that need ephemeral settings (transient command-line
//! overrides, unit tests, layered overrides on top of a file store that
//! must not be written back).
//!
//! # Storage layout
//!
//! The internal representation is
//!
//! ```text
//! HashMap< section_name, HashMap< key, string_value > >
//! ```
//!
//! i.e. the same shape as the C++
//! `UnorderedStringMap< UnorderedStringMap< std::string > >`. Every
//! value is stored in its string-serialised form (`std::to_string` on
//! the way in, `str::parse` on the way out), mirroring the original's
//! `StringUtil::FromChars<T>` round-trip. This makes the storage layer
//! agnostic to the type at the getter and keeps the FFI surface simple.
//!
//! Because the per-section map is a plain `HashMap` (not a multimap),
//! string-list semantics collapse to "last write wins" for `SetStringList`,
//! `AddToStringList`, and `RemoveFromStringList`. That matches the
//! `UnorderedStringMap` alias chosen by the task description; the
//! multimap variant would have been `UnorderedStringMultimap`.
//!
//! # `Save`
//!
//! The C++ version reports `"Memory settings cannot be saved."` from
//! `Save()` because there is no persistent backing store. The task
//! brief explicitly specifies a no-op `save` for the Rust port, so
//! [`MemorySettingsInterface::save`] returns `Ok(())`. The trait still
//! returns `Result<(), Box<dyn Error>>` so that file-backed
//! implementations can return real I/O errors without changing the
//! trait surface.

#![allow(clippy::all)]

use std::collections::HashMap;
// We use `std::error::Error` via the full path to avoid confl icting
// with `settings_interface::Error` when both are glob-re-exported.
use std::ffi::{c_char, CStr};
use std::ptr;

// ---------------------------------------------------------------------------
// `SettingsInterface` trait
//
// Mirrors `common/SettingsInterface.h`. Method names map 1:1 to the C++
// virtuals. The biggest ergonomic divergences are:
//   * Readers return `Option<T>` instead of filling an out-parameter
//     and returning `bool`. Callers use `.unwrap_or(...)` for defaults.
//   * `Save` returns `Result<(), Box<dyn Error>>` so backends can surface
//     real I/O errors instead of writing through an out-parameter.
// ---------------------------------------------------------------------------

/// Abstract configuration backend. Mirrors `SettingsInterface` in the C++
/// tree (`common/SettingsInterface.h`).
///
/// Implementors are responsible for translating between Rust-typed values
/// and whatever on-disk or in-memory representation they store. For the
/// in-memory variant used here ([`MemorySettingsInterface`]) that
/// translation is just `to_string` on the way in and `str::parse` on the
/// way out.
pub trait SettingsInterface {
    /// Persist the in-memory state to the underlying storage. Backends
    /// without a persistent store (e.g. [`MemorySettingsInterface`])
    /// return `Ok(())`.
    fn save(&mut self) -> Result<(), Box<dyn std::error::Error>>;

    /// Drop every section and every value.
    fn clear(&mut self);

    /// `true` if the store is empty (no sections at all).
    fn is_empty(&self) -> bool;

    // -- Typed getters ----------------------------------------------------

    /// Look up a signed 32-bit integer. Returns `None` if the section /
    /// key is missing or the stored value does not parse as `i32`.
    fn get_int(&self, section: &str, key: &str) -> Option<i32>;

    /// Look up an unsigned 32-bit integer.
    fn get_uint(&self, section: &str, key: &str) -> Option<u32>;

    /// Look up a 32-bit float.
    fn get_float(&self, section: &str, key: &str) -> Option<f32>;

    /// Look up a 64-bit float.
    fn get_double(&self, section: &str, key: &str) -> Option<f64>;

    /// Look up a boolean. Accepts `str::parse` truthy / falsy strings.
    fn get_bool(&self, section: &str, key: &str) -> Option<bool>;

    /// Look up a string value.
    fn get_string(&self, section: &str, key: &str) -> Option<String>;

    // -- Typed setters ----------------------------------------------------

    fn set_int(&mut self, section: &str, key: &str, value: i32);
    fn set_uint(&mut self, section: &str, key: &str, value: u32);
    fn set_float(&mut self, section: &str, key: &str, value: f32);
    fn set_double(&mut self, section: &str, key: &str, value: f64);
    fn set_bool(&mut self, section: &str, key: &str, value: bool);
    fn set_string(&mut self, section: &str, key: &str, value: &str);

    // -- String lists -----------------------------------------------------

    /// Return every value associated with `(section, key)`. For a
    /// single-key map like [`MemorySettingsInterface`] this is either
    /// an empty vector or a one-element vector.
    fn get_string_list(&self, section: &str, key: &str) -> Vec<String>;

    /// Replace the list of values for `(section, key)`. With a
    /// single-value map, this keeps only the last item (or deletes the
    /// key if `items` is empty).
    fn set_string_list(&mut self, section: &str, key: &str, items: &[&str]);

    /// Add `item` to the list at `(section, key)`. Returns `true` if a
    /// new entry was inserted; `false` if the item was already present.
    fn add_to_string_list(&mut self, section: &str, key: &str, item: &str) -> bool;

    /// Remove `item` from the list at `(section, key)`. Returns `true`
    /// if an entry was removed.
    fn remove_from_string_list(&mut self, section: &str, key: &str, item: &str) -> bool;

    // -- Section helpers --------------------------------------------------

    /// Snapshot every `(key, value)` pair in `section` as a `Vec`.
    fn get_key_value_list(&self, section: &str) -> Vec<(String, String)>;

    /// Replace every `(key, value)` pair in `section` with `items`.
    fn set_key_value_list(&mut self, section: &str, items: &[(String, String)]);

    /// `true` if `(section, key)` is present (regardless of value).
    fn contains_value(&self, section: &str, key: &str) -> bool;

    /// Remove a single `(section, key)` entry. No-op if absent.
    fn delete_value(&mut self, section: &str, key: &str);

    /// Drop every value from `section` but keep the section itself.
    fn clear_section(&mut self, section: &str);

    /// Remove `section` and all of its contents.
    fn remove_section(&mut self, section: &str);

    /// Drop every section that has no remaining keys.
    fn remove_empty_sections(&mut self);
}

// ---------------------------------------------------------------------------
// `MemorySettingsInterface`
// ---------------------------------------------------------------------------

/// In-memory [`SettingsInterface`] implementation.
///
/// All state lives in [`self.sections`](Self::sections). There is no
/// persistent backing; [`save`](SettingsInterface::save) is a no-op
/// per the task brief. A `Default` instance has zero sections.
#[derive(Debug, Default)]
pub struct MemorySettingsInterface {
    /// Section name -> (key -> stringified value).
    sections: HashMap<String, HashMap<String, String>>,
}

impl MemorySettingsInterface {
    /// Construct a new, empty in-memory settings store.
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up the map for `name`, creating an empty one if missing.
    /// Internal helper used by every setter.
    #[inline]
    fn section_mut(&mut self, name: &str) -> &mut HashMap<String, String> {
        self.sections.entry(name.to_string()).or_default()
    }

    /// Read-only access to the per-section map, or `None` if absent.
    #[inline]
    fn section(&self, name: &str) -> Option<&HashMap<String, String>> {
        self.sections.get(name)
    }
}

impl SettingsInterface for MemorySettingsInterface {
    /// No persistent backing; this is a no-op success.
    fn save(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }

    fn clear(&mut self) {
        self.sections.clear();
    }

    fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    fn get_int(&self, section: &str, key: &str) -> Option<i32> {
        self.section(section)?.get(key)?.parse().ok()
    }

    fn get_uint(&self, section: &str, key: &str) -> Option<u32> {
        self.section(section)?.get(key)?.parse().ok()
    }

    fn get_float(&self, section: &str, key: &str) -> Option<f32> {
        self.section(section)?.get(key)?.parse().ok()
    }

    fn get_double(&self, section: &str, key: &str) -> Option<f64> {
        self.section(section)?.get(key)?.parse().ok()
    }

    fn get_bool(&self, section: &str, key: &str) -> Option<bool> {
        self.section(section)?.get(key)?.parse().ok()
    }

    fn get_string(&self, section: &str, key: &str) -> Option<String> {
        self.section(section)?.get(key).cloned()
    }

    fn set_int(&mut self, section: &str, key: &str, value: i32) {
        self.section_mut(section)
            .insert(key.to_string(), value.to_string());
    }

    fn set_uint(&mut self, section: &str, key: &str, value: u32) {
        self.section_mut(section)
            .insert(key.to_string(), value.to_string());
    }

    fn set_float(&mut self, section: &str, key: &str, value: f32) {
        self.section_mut(section)
            .insert(key.to_string(), value.to_string());
    }

    fn set_double(&mut self, section: &str, key: &str, value: f64) {
        self.section_mut(section)
            .insert(key.to_string(), value.to_string());
    }

    fn set_bool(&mut self, section: &str, key: &str, value: bool) {
        self.section_mut(section)
            .insert(key.to_string(), value.to_string());
    }

    fn set_string(&mut self, section: &str, key: &str, value: &str) {
        self.section_mut(section)
            .insert(key.to_string(), value.to_string());
    }

    fn get_string_list(&self, section: &str, key: &str) -> Vec<String> {
        match self.section(section).and_then(|m| m.get(key)) {
            Some(v) => vec![v.clone()],
            None => Vec::new(),
        }
    }

    fn set_string_list(&mut self, section: &str, key: &str, items: &[&str]) {
        match items.last() {
            Some(last) => self.set_string(section, key, last),
            None => self.delete_value(section, key),
        }
    }

    fn add_to_string_list(&mut self, section: &str, key: &str, item: &str) -> bool {
        if self
            .section(section)
            .and_then(|m| m.get(key))
            .map_or(false, |v| v == item)
        {
            return false;
        }
        self.set_string(section, key, item);
        true
    }

    fn remove_from_string_list(&mut self, section: &str, key: &str, item: &str) -> bool {
        if self
            .section(section)
            .and_then(|m| m.get(key))
            .map_or(false, |v| v == item)
        {
            self.delete_value(section, key);
            return true;
        }
        false
    }

    fn get_key_value_list(&self, section: &str) -> Vec<(String, String)> {
        match self.section(section) {
            Some(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            None => Vec::new(),
        }
    }

    fn set_key_value_list(&mut self, section: &str, items: &[(String, String)]) {
        let map = self.section_mut(section);
        map.clear();
        for (k, v) in items {
            map.insert(k.clone(), v.clone());
        }
    }

    fn contains_value(&self, section: &str, key: &str) -> bool {
        self.section(section).map_or(false, |m| m.contains_key(key))
    }

    fn delete_value(&mut self, section: &str, key: &str) {
        if let Some(m) = self.sections.get_mut(section) {
            m.remove(key);
        }
    }

    fn clear_section(&mut self, section: &str) {
        if let Some(m) = self.sections.get_mut(section) {
            m.clear();
        }
    }

    fn remove_section(&mut self, section: &str) {
        self.sections.remove(section);
    }

    fn remove_empty_sections(&mut self) {
        self.sections.retain(|_, m| !m.is_empty());
    }
}

// ---------------------------------------------------------------------------
// FFI surface
//
// C++ constructs a `MemorySettingsInterface` via the opaque handle
// returned by `pcsx2_memory_settings_create`, then calls individual
// setters / getters. Strings cross the boundary as `*const c_char`
// (NUL-terminated). Output strings use a caller-provided buffer with an
// explicit length so the C++ side controls allocation.
//
// All exported functions are null-tolerant: passing a null handle (or
// null section/key, where applicable) is a no-op rather than UB.
// ---------------------------------------------------------------------------

/// Allocate a new, empty [`MemorySettingsInterface`]. The caller owns
/// the returned pointer and must release it exactly once with
/// [`pcsx2_memory_settings_destroy`].
#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_create() -> *mut MemorySettingsInterface {
    Box::into_raw(Box::new(MemorySettingsInterface::new()))
}

/// Free a handle previously returned by [`pcsx2_memory_settings_create`].
/// Passing a null pointer is a no-op.
#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_destroy(s: *mut MemorySettingsInterface) {
    if s.is_null() {
        return;
    }
    // SAFETY: caller guarantees `s` was produced by
    // `pcsx2_memory_settings_create` and is not aliased.
    unsafe {
        drop(Box::from_raw(s));
    }
}

// -- int32 -----------------------------------------------------------------

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_set_int(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
    key: *const c_char,
    value: i32,
) {
    if s.is_null() || section.is_null() || key.is_null() {
        return;
    }
    // SAFETY: caller guarantees `section` and `key` are valid NUL-
    // terminated C strings for the duration of the call.
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy().into_owned();
        let k = CStr::from_ptr(key).to_string_lossy().into_owned();
        (*s).set_int(&sec, &k, value);
    }
}

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_get_int(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
    key: *const c_char,
    out: *mut i32,
) -> bool {
    if s.is_null() || section.is_null() || key.is_null() || out.is_null() {
        return false;
    }
    // SAFETY: see `pcsx2_memory_settings_set_int`. `out` is a writable
    // pointer the caller owns.
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy();
        let k = CStr::from_ptr(key).to_string_lossy();
        match (*s).get_int(&sec, &k) {
            Some(v) => {
                *out = v;
                true
            }
            None => false,
        }
    }
}

// -- uint32 ----------------------------------------------------------------

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_set_uint(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
    key: *const c_char,
    value: u32,
) {
    if s.is_null() || section.is_null() || key.is_null() {
        return;
    }
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy().into_owned();
        let k = CStr::from_ptr(key).to_string_lossy().into_owned();
        (*s).set_uint(&sec, &k, value);
    }
}

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_get_uint(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
    key: *const c_char,
    out: *mut u32,
) -> bool {
    if s.is_null() || section.is_null() || key.is_null() || out.is_null() {
        return false;
    }
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy();
        let k = CStr::from_ptr(key).to_string_lossy();
        match (*s).get_uint(&sec, &k) {
            Some(v) => {
                *out = v;
                true
            }
            None => false,
        }
    }
}

// -- float -----------------------------------------------------------------

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_set_float(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
    key: *const c_char,
    value: f32,
) {
    if s.is_null() || section.is_null() || key.is_null() {
        return;
    }
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy().into_owned();
        let k = CStr::from_ptr(key).to_string_lossy().into_owned();
        (*s).set_float(&sec, &k, value);
    }
}

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_get_float(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
    key: *const c_char,
    out: *mut f32,
) -> bool {
    if s.is_null() || section.is_null() || key.is_null() || out.is_null() {
        return false;
    }
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy();
        let k = CStr::from_ptr(key).to_string_lossy();
        match (*s).get_float(&sec, &k) {
            Some(v) => {
                *out = v;
                true
            }
            None => false,
        }
    }
}

// -- double ----------------------------------------------------------------

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_set_double(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
    key: *const c_char,
    value: f64,
) {
    if s.is_null() || section.is_null() || key.is_null() {
        return;
    }
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy().into_owned();
        let k = CStr::from_ptr(key).to_string_lossy().into_owned();
        (*s).set_double(&sec, &k, value);
    }
}

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_get_double(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
    key: *const c_char,
    out: *mut f64,
) -> bool {
    if s.is_null() || section.is_null() || key.is_null() || out.is_null() {
        return false;
    }
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy();
        let k = CStr::from_ptr(key).to_string_lossy();
        match (*s).get_double(&sec, &k) {
            Some(v) => {
                *out = v;
                true
            }
            None => false,
        }
    }
}

// -- bool ------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_set_bool(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
    key: *const c_char,
    value: bool,
) {
    if s.is_null() || section.is_null() || key.is_null() {
        return;
    }
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy().into_owned();
        let k = CStr::from_ptr(key).to_string_lossy().into_owned();
        (*s).set_bool(&sec, &k, value);
    }
}

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_get_bool(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
    key: *const c_char,
    out: *mut bool,
) -> bool {
    if s.is_null() || section.is_null() || key.is_null() || out.is_null() {
        return false;
    }
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy();
        let k = CStr::from_ptr(key).to_string_lossy();
        match (*s).get_bool(&sec, &k) {
            Some(v) => {
                *out = v;
                true
            }
            None => false,
        }
    }
}

// -- string ----------------------------------------------------------------

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_set_string(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
    key: *const c_char,
    value: *const c_char,
) {
    if s.is_null() || section.is_null() || key.is_null() || value.is_null() {
        return;
    }
    // SAFETY: `section`, `key`, `value` are valid NUL-terminated C
    // strings for the duration of the call.
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy().into_owned();
        let k = CStr::from_ptr(key).to_string_lossy().into_owned();
        let v = CStr::from_ptr(value).to_string_lossy().into_owned();
        (*s).set_string(&sec, &k, &v);
    }
}

/// Copy the stored string for `(section, key)` into `buf`, NUL-terminating
/// it. `buf_len` is the capacity of `buf` in bytes (including the
/// trailing NUL slot).
///
/// Returns:
/// * `> 0` — the number of bytes written, **excluding** the trailing
///   NUL. The buffer is NUL-terminated on success.
/// * `-1`  — the key is missing, the buffer is too small, or any input
///   pointer is null / capacity is zero. On failure the buffer is left
///   untouched so the caller can detect truncation by comparing the
///   returned length to its original buffer size.
#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_get_string(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
    key: *const c_char,
    buf: *mut c_char,
    buf_len: usize,
) -> i64 {
    if s.is_null() || section.is_null() || key.is_null() || buf.is_null() || buf_len == 0 {
        return -1;
    }
    // SAFETY: `section` and `key` are valid NUL-terminated C strings.
    // `buf` is a writable buffer of `buf_len` bytes.
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy();
        let k = CStr::from_ptr(key).to_string_lossy();
        let Some(value) = (*s).get_string(&sec, &k) else {
            return -1;
        };
        let bytes = value.as_bytes();
        // Need room for bytes + NUL terminator.
        if bytes.len() + 1 > buf_len {
            return -1;
        }
        ptr::copy_nonoverlapping(bytes.as_ptr(), buf as *mut u8, bytes.len());
        // Write the trailing NUL.
        *buf.add(bytes.len()) = 0;
        bytes.len() as i64
    }
}

// -- store-level helpers ---------------------------------------------------

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_clear(s: *mut MemorySettingsInterface) {
    if s.is_null() {
        return;
    }
    unsafe {
        (*s).clear();
    }
}

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_is_empty(s: *mut MemorySettingsInterface) -> bool {
    if s.is_null() {
        return true;
    }
    unsafe { (*s).is_empty() }
}

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_contains_value(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
    key: *const c_char,
) -> bool {
    if s.is_null() || section.is_null() || key.is_null() {
        return false;
    }
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy();
        let k = CStr::from_ptr(key).to_string_lossy();
        (*s).contains_value(&sec, &k)
    }
}

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_delete_value(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
    key: *const c_char,
) {
    if s.is_null() || section.is_null() || key.is_null() {
        return;
    }
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy().into_owned();
        let k = CStr::from_ptr(key).to_string_lossy().into_owned();
        (*s).delete_value(&sec, &k);
    }
}

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_clear_section(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
) {
    if s.is_null() || section.is_null() {
        return;
    }
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy().into_owned();
        (*s).clear_section(&sec);
    }
}

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_remove_section(
    s: *mut MemorySettingsInterface,
    section: *const c_char,
) {
    if s.is_null() || section.is_null() {
        return;
    }
    unsafe {
        let sec = CStr::from_ptr(section).to_string_lossy().into_owned();
        (*s).remove_section(&sec);
    }
}

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_remove_empty_sections(s: *mut MemorySettingsInterface) {
    if s.is_null() {
        return;
    }
    unsafe {
        (*s).remove_empty_sections();
    }
}

#[no_mangle]
pub extern "C" fn pcsx2_memory_settings_save(s: *mut MemorySettingsInterface) -> bool {
    if s.is_null() {
        return false;
    }
    // SAFETY: caller guarantees a live handle.
    unsafe { (*s).save().is_ok() }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_round_trip() {
        let mut s = MemorySettingsInterface::new();
        s.set_int("General", "frameskip", 3);
        s.set_uint("General", "ratio", 4);
        s.set_float("General", "speed", 1.5);
        s.set_double("General", "scale", 2.0);
        s.set_bool("General", "widescreen", true);
        s.set_string("General", "title", "PCSX2");

        assert_eq!(s.get_int("General", "frameskip"), Some(3));
        assert_eq!(s.get_uint("General", "ratio"), Some(4));
        assert_eq!(s.get_float("General", "speed"), Some(1.5));
        assert_eq!(s.get_double("General", "scale"), Some(2.0));
        assert_eq!(s.get_bool("General", "widescreen"), Some(true));
        assert_eq!(
            s.get_string("General", "title"),
            Some("PCSX2".to_string())
        );
    }

    #[test]
    fn missing_section_returns_none() {
        let s = MemorySettingsInterface::new();
        assert!(s.get_int("None", "x").is_none());
        assert!(s.get_string("None", "x").is_none());
    }

    #[test]
    fn clear_and_is_empty() {
        let mut s = MemorySettingsInterface::new();
        s.set_int("S", "k", 1);
        assert!(!s.is_empty());
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn delete_and_remove_section() {
        let mut s = MemorySettingsInterface::new();
        s.set_int("A", "k1", 1);
        s.set_int("A", "k2", 2);
        s.delete_value("A", "k1");
        assert!(!s.contains_value("A", "k1"));
        assert!(s.contains_value("A", "k2"));
        s.remove_section("A");
        assert!(s.is_empty());
    }

    #[test]
    fn clear_section_keeps_section() {
        let mut s = MemorySettingsInterface::new();
        s.set_int("A", "k", 1);
        s.clear_section("A");
        // Section is still present, but empty.
        assert!(!s.is_empty());
        assert!(!s.contains_value("A", "k"));
        s.remove_empty_sections();
        assert!(s.is_empty());
    }

    #[test]
    fn remove_empty_sections() {
        let mut s = MemorySettingsInterface::new();
        s.set_int("A", "k", 1);
        s.delete_value("A", "k");
        s.remove_empty_sections();
        assert!(s.is_empty());
    }

    #[test]
    fn key_value_list_round_trip() {
        let mut s = MemorySettingsInterface::new();
        let items = vec![
            ("a".to_string(), "1".to_string()),
            ("b".to_string(), "2".to_string()),
        ];
        s.set_key_value_list("S", &items);
        let mut got = s.get_key_value_list("S");
        got.sort();
        let mut want = items.clone();
        want.sort();
        assert_eq!(got, want);
    }

    #[test]
    fn string_list_collapse() {
        let mut s = MemorySettingsInterface::new();
        assert!(s.add_to_string_list("P", "n", "a"));
        assert!(!s.add_to_string_list("P", "n", "a"));
        assert_eq!(s.get_string_list("P", "n"), vec!["a".to_string()]);
        assert!(s.remove_from_string_list("P", "n", "a"));
        assert!(!s.remove_from_string_list("P", "n", "a"));
    }

    #[test]
    fn save_is_noop() {
        let mut s = MemorySettingsInterface::new();
        s.set_int("S", "k", 1);
        assert!(s.save().is_ok());
        // Data still there after save.
        assert_eq!(s.get_int("S", "k"), Some(1));
    }

    #[test]
    fn ffi_roundtrip_int() {
        unsafe {
            let s = pcsx2_memory_settings_create();
            assert!(!s.is_null());
            pcsx2_memory_settings_set_int(
                s,
                b"A\0".as_ptr() as *const c_char,
                b"k\0".as_ptr() as *const c_char,
                42,
            );
            let mut out: i32 = 0;
            assert!(pcsx2_memory_settings_get_int(
                s,
                b"A\0".as_ptr() as *const c_char,
                b"k\0".as_ptr() as *const c_char,
                &mut out
            ));
            assert_eq!(out, 42);
            pcsx2_memory_settings_destroy(s);
        }
    }

    #[test]
    fn ffi_roundtrip_string() {
        unsafe {
            let s = pcsx2_memory_settings_create();
            pcsx2_memory_settings_set_string(
                s,
                b"A\0".as_ptr() as *const c_char,
                b"k\0".as_ptr() as *const c_char,
                b"hi\0".as_ptr() as *const c_char,
            );
            let mut buf = [0i8; 16];
            let n = pcsx2_memory_settings_get_string(
                s,
                b"A\0".as_ptr() as *const c_char,
                b"k\0".as_ptr() as *const c_char,
                buf.as_mut_ptr(),
                buf.len(),
            );
            assert_eq!(n, 2);
            let s_buf = CStr::from_ptr(buf.as_ptr()).to_str().unwrap();
            assert_eq!(s_buf, "hi");
            pcsx2_memory_settings_destroy(s);
        }
    }

    #[test]
    fn ffi_string_buffer_too_small() {
        unsafe {
            let s = pcsx2_memory_settings_create();
            pcsx2_memory_settings_set_string(
                s,
                b"A\0".as_ptr() as *const c_char,
                b"k\0".as_ptr() as *const c_char,
                b"hello\0".as_ptr() as *const c_char,
            );
            // Buffer only fits 2 bytes of payload + NUL.
            let mut buf = [0i8; 3];
            let n = pcsx2_memory_settings_get_string(
                s,
                b"A\0".as_ptr() as *const c_char,
                b"k\0".as_ptr() as *const c_char,
                buf.as_mut_ptr(),
                buf.len(),
            );
            assert_eq!(n, -1);
            pcsx2_memory_settings_destroy(s);
        }
    }

    #[test]
    fn ffi_null_is_safe() {
        unsafe {
            pcsx2_memory_settings_destroy(ptr::null_mut());
            pcsx2_memory_settings_clear(ptr::null_mut());
            assert!(pcsx2_memory_settings_is_empty(ptr::null_mut()));
            assert!(!pcsx2_memory_settings_save(ptr::null_mut()));
        }
    }
}
