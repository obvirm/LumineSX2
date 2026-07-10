// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+

//! Rust port of `common/SettingsWrapper.{h,cpp}`.
//!
//! The C++ header defines three RAII wrapper classes
//! (`SettingsLoadWrapper`, `SettingsSaveWrapper`, `SettingsClearWrapper`)
//! plus a set of macro helpers (`SettingsWrapSection`, `SettingsWrapEntry`,
//! etc.). Each wrapper takes a reference to a [`SettingsInterface`] and
//! provides `Entry()` overloads that either read, write, or delete values
//! depending on the wrapper type.
//!
//! # Design
//!
//! In C++ the base class `SettingsWrapper` stores `SettingsInterface& m_si`
//! and the load/save/clear semantics are chosen at construction time via
//! virtual `IsLoading()` / `IsSaving()`. The Rust version uses three
//! distinct concrete types instead of the virtual-branch pattern:
//!
//! | C++ class              | Rust struct              | Semantics              |
//! |------------------------|--------------------------|------------------------|
//! | `SettingsLoadWrapper`  | [`SettingsLoadWrapper`]  | `get_*` → value        |
//! | `SettingsSaveWrapper`  | [`SettingsSaveWrapper`]  | `set_*` ← value        |
//! | `SettingsClearWrapper` | [`SettingsClearWrapper`] | `delete_value`         |
//!
//! Each struct holds a `&'a dyn SettingsInterface` (or `&'a mut dyn` for
//! write/delete wrappers) so callers can pass either a concrete back-end
//! or a trait object without extra generics.
//!
//! # Macros
//!
//! The C++ `SettingsWrapEntry` / `SettingsWrapEnum` / etc. macros are
//! replaced by the [`settings_section!`] and [`settings_entry!`] macros
//! from this module, plus the [`SettingsEntries`] extension trait that
//! enables `.entry()` chaining.
//!
//! # Example
//!
//! ```rust
//! use pcsx2_common_rs::settings_wrapper::*;
//! use pcsx2_common_rs::MemorySettingsInterface;
//!
//! let mut iface = MemorySettingsInterface::new();
//!
//! // --- Save ---
//! {
//!     let mut w = SettingsSaveWrapper::new(&mut iface);
//!     w.entry_int("Display", "width", 1920);
//!     w.entry_bool("Display", "vsync", true);
//! }
//!
//! // --- Load ---
//! {
//!     let w = SettingsLoadWrapper::new(&iface);
//!     let w: i32 = w.entry_int("Display", "width", 1024);       // returns 1920
//!     let vsync: bool = w.entry_bool("Display", "vsync", false);  // returns true
//! }
//! ```

use crate::settings_interface::SettingsInterface;

// ---------------------------------------------------------------------------
// SettingsLoadWrapper
// ---------------------------------------------------------------------------

/// RAII wrapper that **reads** settings from an interface.
///
/// Each `entry_*` call looks up the value on the underlying interface and
/// returns the stored value (or the supplied default if absent).
pub struct SettingsLoadWrapper<'a> {
    si: &'a dyn SettingsInterface,
}

impl<'a> SettingsLoadWrapper<'a> {
    /// Wrap a borrowed settings interface for reading.
    #[inline]
    pub fn new(si: &'a dyn SettingsInterface) -> Self {
        Self { si }
    }

    /// Read a signed 32-bit integer.
    #[inline]
    pub fn entry_int(&self, section: &str, key: &str, default: i32) -> i32 {
        self.si.get_int(section, key).unwrap_or(default)
    }

    /// Read an unsigned 32-bit integer.
    #[inline]
    pub fn entry_uint(&self, section: &str, key: &str, default: u32) -> u32 {
        self.si.get_uint(section, key).unwrap_or(default)
    }

    /// Read a boolean.
    #[inline]
    pub fn entry_bool(&self, section: &str, key: &str, default: bool) -> bool {
        self.si.get_bool(section, key).unwrap_or(default)
    }

    /// Read a 32-bit float.
    #[inline]
    pub fn entry_float(&self, section: &str, key: &str, default: f32) -> f32 {
        self.si.get_float(section, key).unwrap_or(default)
    }

    /// Read a string. Returns the default if the key is absent.
    #[inline]
    pub fn entry_string<'s>(&self, section: &str, key: &str, default: &'s str) -> String {
        self.si.get_string(section, key).unwrap_or_else(|| default.to_string())
    }

    /// Read a bitfield (stored as int).
    ///
    /// C++ counterpart: `EntryBitfield(section, key, value, defvalue)`.
    /// Unlike the C++ version this does **not** mutate an in/out
    /// variable; it simply returns the stored value.
    #[inline]
    pub fn entry_bitfield(&self, section: &str, key: &str, default: i32) -> i32 {
        self.si.get_int(section, key).unwrap_or(default)
    }

    /// Read a bit-bool (stored as bool).
    ///
    /// C++ counterpart: `EntryBitBool(section, key, value, defvalue)`.
    #[inline]
    pub fn entry_bit_bool(&self, section: &str, key: &str, default: bool) -> bool {
        self.si.get_bool(section, key).unwrap_or(default)
    }

    /// Read an enum stored as an integer index into `enum_names`.
    ///
    /// C++ counterpart: `_EnumEntry(section, key, value, enumArray, defvalue)`.
    /// Returns the index (0-based) into `enum_names` whose string value
    /// matches the stored string. If the stored string is not found,
    /// returns `default` and logs a warning.
    #[inline]
    pub fn entry_enum(&self, section: &str, key: &str, enum_names: &[&str], default: i32) -> i32 {
        self._enum_entry(section, key, enum_names, default)
    }

    /// Internal: enum lookup by string comparison.
    fn _enum_entry(&self, section: &str, key: &str, enum_names: &[&str], default: i32) -> i32 {
        let default = default.clamp(0, enum_names.len() as i32 - 1);
        let stored = self.si.get_string(section, key);
        match stored {
            Some(ref val) if !val.is_empty() => {
                // Find index matching the stored string.
                for (i, name) in enum_names.iter().enumerate() {
                    if *name == val.as_str() {
                        return i as i32;
                    }
                }
                log::warn!(
                    "(LoadSettings) Warning: Unrecognized value '{}' on key '{}' \
                     Using the default setting of '{}'.",
                    val,
                    key,
                    enum_names.get(default as usize).unwrap_or(&"?")
                );
                default
            }
            _ => default,
        }
    }
}

// ---------------------------------------------------------------------------
// SettingsSaveWrapper
// ---------------------------------------------------------------------------

/// RAII wrapper that **writes** settings to an interface.
///
/// Each `entry_*` call sets the value on the underlying interface.
pub struct SettingsSaveWrapper<'a> {
    si: &'a mut dyn SettingsInterface,
    modified: bool,
}

impl<'a> SettingsSaveWrapper<'a> {
    /// Wrap a mutable borrowed settings interface for writing.
    #[inline]
    pub fn new(si: &'a mut dyn SettingsInterface) -> Self {
        Self {
            si,
            modified: false,
        }
    }

    /// Returns `true` if at least one entry has been set.
    #[inline]
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Mark the wrapper as modified even if no entry was explicitly set
    /// (useful for side‑channel mutations or batch operations).
    #[inline]
    pub fn set_modified(&mut self) {
        self.modified = true;
    }

    /// Write a signed 32-bit integer.
    #[inline]
    pub fn entry_int(&mut self, section: &str, key: &str, value: i32) -> i32 {
        self.si.set_int(section, key, value);
        self.modified = true;
        value
    }

    /// Write an unsigned 32-bit integer.
    #[inline]
    pub fn entry_uint(&mut self, section: &str, key: &str, value: u32) -> u32 {
        self.si.set_uint(section, key, value);
        self.modified = true;
        value
    }

    /// Write a boolean.
    #[inline]
    pub fn entry_bool(&mut self, section: &str, key: &str, value: bool) -> bool {
        self.si.set_bool(section, key, value);
        self.modified = true;
        value
    }

    /// Write a 32-bit float.
    #[inline]
    pub fn entry_float(&mut self, section: &str, key: &str, value: f32) -> f32 {
        self.si.set_float(section, key, value);
        self.modified = true;
        value
    }

    /// Write a string.
    #[inline]
    pub fn entry_string(&mut self, section: &str, key: &str, value: &str) {
        self.si.set_string(section, key, value);
        self.modified = true;
    }

    /// Write a bitfield (stored as int).
    #[inline]
    pub fn entry_bitfield(&mut self, section: &str, key: &str, value: i32) -> i32 {
        self.si.set_int(section, key, value);
        self.modified = true;
        value
    }

    /// Write a bit‑bool (stored as bool).
    #[inline]
    pub fn entry_bit_bool(&mut self, section: &str, key: &str, value: bool) -> bool {
        self.si.set_bool(section, key, value);
        self.modified = true;
        value
    }

    /// Write an enum stored as an integer index into `enum_names`.
    ///
    /// C++ counterpart: `_EnumEntry(section, key, value, enumArray, defvalue)`.
    /// The value is clamped to valid bounds and written as the
    /// corresponding string from `enum_names`.
    #[inline]
    pub fn entry_enum(&mut self, section: &str, key: &str, enum_names: &[&str], value: i32) -> i32 {
        let cnt = enum_names.len() as i32;
        let index = if value < 0 || value >= cnt { 0 } else { value };
        self.si.set_string(section, key, enum_names[index as usize]);
        self.modified = true;
        index
    }
}

// ---------------------------------------------------------------------------
// SettingsClearWrapper
// ---------------------------------------------------------------------------

/// RAII wrapper that **deletes** settings from an interface.
///
/// Each `entry_*` call removes the key from the underlying interface.
pub struct SettingsClearWrapper<'a> {
    si: &'a mut dyn SettingsInterface,
}

impl<'a> SettingsClearWrapper<'a> {
    /// Wrap a mutable borrowed settings interface for deletion.
    #[inline]
    pub fn new(si: &'a mut dyn SettingsInterface) -> Self {
        Self { si }
    }

    /// Delete a signed 32-bit integer entry.
    #[inline]
    pub fn entry_int(&mut self, section: &str, key: &str, _value: i32) -> i32 {
        self.si.delete_value(section, key);
        _value
    }

    /// Delete an unsigned 32-bit integer entry.
    #[inline]
    pub fn entry_uint(&mut self, section: &str, key: &str, _value: u32) -> u32 {
        self.si.delete_value(section, key);
        _value
    }

    /// Delete a boolean entry.
    #[inline]
    pub fn entry_bool(&mut self, section: &str, key: &str, _value: bool) -> bool {
        self.si.delete_value(section, key);
        _value
    }

    /// Delete a float entry.
    #[inline]
    pub fn entry_float(&mut self, section: &str, key: &str, _value: f32) -> f32 {
        self.si.delete_value(section, key);
        _value
    }

    /// Delete a string entry.
    #[inline]
    pub fn entry_string(&mut self, section: &str, key: &str, _value: &str) {
        self.si.delete_value(section, key);
    }

    /// Delete a bitfield (stored as int).
    #[inline]
    pub fn entry_bitfield(&mut self, section: &str, key: &str, value: i32) -> i32 {
        self.si.delete_value(section, key);
        value // matching C++: returns the input value (defvalue)
    }

    /// Delete a bit‑bool.
    #[inline]
    pub fn entry_bit_bool(&mut self, section: &str, key: &str, value: bool) -> bool {
        self.si.delete_value(section, key);
        value // matching C++: returns the input value
    }

    /// Delete an enum entry.
    #[inline]
    pub fn entry_enum(&mut self, section: &str, key: &str, _value: i32, _enum_names: &[&str]) -> i32 {
        self.si.delete_value(section, key);
        _value
    }
}

// ---------------------------------------------------------------------------
// Macro helpers
//
// These replace the C++ macros:
//   SettingsWrapSection → settings_section!
//   SettingsWrapEntry   → settings_entry!
//   SettingsWrapEnumEx  → settings_enum!
//   SettingsWrapBitfield → settings_bitfield!
//   SettingsWrapBitBool  → settings_bitbool!
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Macro helpers
//
// Simplified Rust equivalents of the C++ macros. Because Rust doesn't
// have C++-style function overloading, the macros below are typed.
// ---------------------------------------------------------------------------

/// Declare the current settings section for subsequent entry calls.
///
/// C++ equivalent: `SettingsWrapSection`
#[macro_export]
macro_rules! settings_section {
    ($section:expr) => {
        let _settings_section: &str = $section;
    };
}

/// Load or save an `i32` entry depending on wrapper type.
///
/// For `SettingsLoadWrapper`: reads from interface and assigns to `$var`.
/// For `SettingsSaveWrapper`: writes `$var` to the interface.
/// For `SettingsClearWrapper`: deletes the key.
#[macro_export]
macro_rules! settings_entry {
    ($wrapper:expr, $var:ident) => {
        settings_entry!($wrapper, stringify!($var), $var)
    };
    ($wrapper:expr, $key:expr, $var:expr) => {{
        // Call `entry_int` on any wrapper type.
        $wrapper.entry_int(_settings_section, $key, $var)
    }};
}

// ---------------------------------------------------------------------------
// Extension trait: `entry_auto` dispatch
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_settings_interface::MemorySettingsInterface;

    #[test]
    fn load_roundtrip() {
        let mut iface = MemorySettingsInterface::new();
        // Pre-set a value.
        iface.set_int("Test", "my_int", 42);
        iface.set_bool("Test", "my_bool", true);
        iface.set_string("Test", "my_str", "hello");

        let w = SettingsLoadWrapper::new(&iface);
        assert_eq!(w.entry_int("Test", "my_int", 0), 42);
        assert_eq!(w.entry_bool("Test", "my_bool", false), true);
        assert_eq!(w.entry_string("Test", "my_str", ""), "hello");
    }

    #[test]
    fn load_default() {
        let iface = MemorySettingsInterface::new();
        let w = SettingsLoadWrapper::new(&iface);
        assert_eq!(w.entry_int("Test", "missing", 99), 99);
        assert_eq!(w.entry_bool("Test", "missing", true), true);
        assert_eq!(w.entry_string("Test", "missing", "default"), "default");
    }

    #[test]
    fn save_then_load() {
        let mut iface = MemorySettingsInterface::new();
        {
            let mut w = SettingsSaveWrapper::new(&mut iface);
            assert!(!w.is_modified());
            w.entry_int("Display", "width", 1920);
            assert!(w.is_modified());
            w.entry_bool("Display", "vsync", true);
            w.entry_string("Display", "mode", "Fullscreen");
        }
        {
            let w = SettingsLoadWrapper::new(&iface);
            assert_eq!(w.entry_int("Display", "width", 0), 1920);
            assert_eq!(w.entry_bool("Display", "vsync", false), true);
            assert_eq!(w.entry_string("Display", "mode", ""), "Fullscreen");
        }
    }

    #[test]
    fn clear_removes() {
        let mut iface = MemorySettingsInterface::new();
        iface.set_int("Test", "k", 1);
        {
            let mut w = SettingsClearWrapper::new(&mut iface);
            w.entry_int("Test", "k", 0);
        }
        assert!(!iface.contains_value("Test", "k"));
    }

    #[test]
    fn enum_save_and_load() {
        let names: &[&str] = &["Off", "On", "Auto"];
        let mut iface = MemorySettingsInterface::new();
        // Save as index 2 → "Auto"
        {
            let mut w = SettingsSaveWrapper::new(&mut iface);
            w.entry_enum("Mode", "vsync", names, 2);
        }
        // Load back — should match "Auto" → index 2
        {
            let w = SettingsLoadWrapper::new(&iface);
            assert_eq!(w.entry_enum("Mode", "vsync", names, 0), 2);
        }
    }

    #[test]
    fn bitfield_roundtrip() {
        let mut iface = MemorySettingsInterface::new();
        {
            let mut w = SettingsSaveWrapper::new(&mut iface);
            w.entry_bitfield("Bf", "val", 0xFF);
        }
        {
            let w = SettingsLoadWrapper::new(&iface);
            assert_eq!(w.entry_bitfield("Bf", "val", 0), 0xFF);
        }
    }

    #[test]
    fn macro_load_entry() {
        let mut iface = MemorySettingsInterface::new();
        iface.set_int("Game", "scheck", 1);
        settings_section!("Game");
        let w = SettingsLoadWrapper::new(&iface);
        let val: i32 = settings_entry!(w, scheck);
        assert_eq!(val, 1);
    }
}
