// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Idiomatic Rust translation of `common/MemorySettingsInterface.{h,cpp}`.
//
// `MemorySettingsInterface` is the in-process analogue of an INI-backed
// configuration store. It implements the same `SettingsInterface` contract
// as the file-backed backends, but keeps every value in process memory.
// Useful for transient overrides, unit tests, and layering cached values
// on top of a file store without writing them back.
//
// Only the `std` crate is used. Strings are stored typed (`Value::String`)
// rather than as raw text, but the getter methods transparently fall back
// to `str::parse` for backwards compatibility with the textual storage
// used by the C++ original, so round-tripping values through a string
// accessor still works.

use std::collections::HashMap;

/// Typed value stored inside a [`Section`].
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// Signed 32-bit integer.
    Int(i32),
    /// 32-bit float.
    Float(f32),
    /// Boolean flag.
    Bool(bool),
    /// Owned string.
    String(String),
}

/// A single configuration section: a flat map from key to typed value.
pub type Section = HashMap<String, Value>;

/// In-memory [`SettingsInterface`](super::SettingsInterface::SettingsInterface) implementation.
///
/// The data is owned by the struct and dropped when the struct is dropped.
/// `save` is intentionally a failure: there is no persistent backing store
/// to write to.
#[derive(Clone, Debug, Default)]
pub struct MemorySettingsInterface {
    /// Section name -> section contents.
    pub sections: HashMap<String, Section>,
}

impl MemorySettingsInterface {
    /// Construct a new, empty in-memory settings store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a mutable handle to the named section, creating an empty
    /// section if one does not already exist.
    fn section_mut(&mut self, name: &str) -> &mut Section {
        self.sections.entry(name.to_string()).or_default()
    }

    /// Look up a single typed value by section and key.
    fn get(&self, section: &str, key: &str) -> Option<&Value> {
        self.sections.get(section)?.get(key)
    }
}

/// Format a [`Value`] for textual use, matching `std::to_string` formatting
/// from the C++ original.
fn value_to_string(v: &Value) -> String {
    match v {
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::String(s) => s.clone(),
    }
}

impl super::SettingsInterface::SettingsInterface for MemorySettingsInterface {
    /// Persist the in-memory state. Always fails: the store has no
    /// persistent backing.
    fn save(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        Err("Memory settings cannot be saved.".into())
    }

    /// Returns `true` if no values are stored at all.
    fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    /// Look up a signed integer value, or `None` if the key is missing or
    /// holds an incompatible type. Falls back to parsing the textual form
    /// if the stored value is a [`Value::String`].
    fn get_int(&self, section: &str, key: &str) -> Option<i32> {
        match self.get(section, key)? {
            Value::Int(v) => Some(*v),
            Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// Look up a floating-point value, or `None` if the key is missing or
    /// holds an incompatible type. Falls back to parsing the textual form
    /// if the stored value is a [`Value::String`].
    fn get_float(&self, section: &str, key: &str) -> Option<f32> {
        match self.get(section, key)? {
            Value::Float(v) => Some(*v),
            Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// Look up a boolean value, or `None` if the key is missing or holds
    /// an incompatible type. Falls back to parsing the textual form if the
    /// stored value is a [`Value::String`].
    fn get_bool(&self, section: &str, key: &str) -> Option<bool> {
        match self.get(section, key)? {
            Value::Bool(v) => Some(*v),
            Value::String(s) => s.parse().ok(),
            _ => None,
        }
    }

    /// Look up a string value, or `None` if the key is missing. Any stored
    /// variant is rendered to text via `value_to_string` to mirror the
    /// C++ behaviour of returning the textual representation regardless of
    /// how the value was originally written.
    fn get_string(&self, section: &str, key: &str) -> Option<String> {
        self.get(section, key).map(value_to_string)
    }

    /// Store a signed integer value.
    fn set_int(&mut self, section: &str, key: &str, value: i32) {
        self.section_mut(section)
            .insert(key.to_string(), Value::Int(value));
    }

    /// Store a floating-point value.
    fn set_float(&mut self, section: &str, key: &str, value: f32) {
        self.section_mut(section)
            .insert(key.to_string(), Value::Float(value));
    }

    /// Store a boolean value.
    fn set_bool(&mut self, section: &str, key: &str, value: bool) {
        self.section_mut(section)
            .insert(key.to_string(), Value::Bool(value));
    }

    /// Store a string value.
    fn set_string(&mut self, section: &str, key: &str, value: &str) {
        self.section_mut(section)
            .insert(key.to_string(), Value::String(value.to_string()));
    }

    /// Create a new (empty) section. Returns `true` if a new section
    /// was added, `false` if it already existed.
    fn add_section(&mut self, section: &str) -> bool {
        self.sections
            .insert(section.to_string(), HashMap::new())
            .is_none()
    }

    /// Remove an entire section. Returns `true` if the section existed
    /// and was removed, `false` otherwise.
    fn remove_section(&mut self, section: &str) -> bool {
        self.sections.remove(section).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::SettingsInterface::SettingsInterface as _;

    #[test]
    fn save_is_unsupported() {
        let mut s = MemorySettingsInterface::new();
        assert!(s.save().is_err());
    }

    #[test]
    fn round_trip_typed_values() {
        let mut s = MemorySettingsInterface::new();
        s.set_int("General", "frameskip", 3);
        s.set_float("General", "speed", 1.5);
        s.set_bool("General", "widescreen", true);
        s.set_string("General", "title", "PCSX2");

        assert_eq!(s.get_int("General", "frameskip"), Some(3));
        assert_eq!(s.get_float("General", "speed"), Some(1.5));
        assert_eq!(s.get_bool("General", "widescreen"), Some(true));
        assert_eq!(s.get_string("General", "title"), Some("PCSX2".to_string()));
    }

    #[test]
    fn get_string_falls_back_to_typed_values() {
        let mut s = MemorySettingsInterface::new();
        s.set_int("S", "n", 42);
        assert_eq!(s.get_string("S", "n"), Some("42".to_string()));
    }

    #[test]
    fn missing_section_or_key_returns_none() {
        let s = MemorySettingsInterface::new();
        assert!(s.get_int("None", "x").is_none());
        assert!(s.get_string("None", "x").is_none());
    }

    #[test]
    fn type_mismatch_returns_none() {
        let mut s = MemorySettingsInterface::new();
        s.set_int("S", "k", 7);
        // bool coercion is not implicit
        assert!(s.get_bool("S", "k").is_none());
    }

    #[test]
    fn add_and_remove_section() {
        let mut s = MemorySettingsInterface::new();
        assert!(s.add_section("A"));
        assert!(!s.add_section("A"));
        assert!(s.remove_section("A"));
        assert!(!s.remove_section("A"));
    }

    #[test]
    fn string_list_round_trip() {
        let mut s = MemorySettingsInterface::new();
        s.set_string_list("Plugins", "names", &["one".to_string(), "two".to_string()]);
        assert_eq!(
            s.get_string_list("Plugins", "names"),
            vec!["one".to_string(), "two".to_string()]
        );
    }

    #[test]
    fn string_list_add_and_remove() {
        let mut s = MemorySettingsInterface::new();
        s.add_to_string_list("P", "n", "a");
        s.add_to_string_list("P", "n", "b");
        // duplicate adds are rejected
        assert!(!s.add_to_string_list("P", "n", "a"));
        assert!(s.remove_from_string_list("P", "n", "a"));
        // removing a missing entry is a no-op
        assert!(!s.remove_from_string_list("P", "n", "missing"));
    }
}
