// SPDX-FileCopyrightText: 2002-2026 PCSX2 Dev Team
// SPDX-License-Identifier: GPL-3.0+
//
// Idiomatic Rust translation of `common/SettingsInterface.h`.
//
// This module defines the abstract `SettingsInterface` trait, which
// is the type-erased contract for reading and writing configuration
// values regardless of the underlying storage backend (INI file,
// command-line overrides, in-memory layer, ...). It also provides a
// `SettingsType` enum that callers can use to discriminate the kind
// of a setting. Only the `std` crate is used.

use std::error::Error;

/// The concrete type of a configuration value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum SettingsType {
    /// Signed integer value (`i32`).
    Int,
    /// Floating-point value (`f32`).
    Float,
    /// Boolean flag.
    Bool,
    /// String value.
    String,
    /// Ordered list of strings.
    List,
}

/// Abstract configuration backend.
///
/// Readers take `&self` and return `Option<T>` to represent "key not
/// present" — callers can supply a default with `.unwrap_or(...)` at
/// the call site, which replaces the C++ overloads that took a
/// `default_value` parameter. Writers and lifecycle methods take
/// `&mut self`. `save` returns a `Result` so backends can surface
/// I/O or parse errors to the caller instead of writing them through
/// an out-parameter the way the C++ original does.
pub trait SettingsInterface {
    /// Persist the in-memory state to the underlying storage.
    fn save(&mut self) -> Result<(), Box<dyn Error>>;

    /// Returns `true` if no values are stored at all.
    fn is_empty(&self) -> bool;

    /// Look up a signed integer value, or `None` if the key is missing.
    fn get_int(&self, section: &str, key: &str) -> Option<i32>;

    /// Look up a floating-point value, or `None` if the key is missing.
    fn get_float(&self, section: &str, key: &str) -> Option<f32>;

    /// Look up a boolean value, or `None` if the key is missing.
    fn get_bool(&self, section: &str, key: &str) -> Option<bool>;

    /// Look up a string value, or `None` if the key is missing.
    fn get_string(&self, section: &str, key: &str) -> Option<String>;

    /// Store a signed integer value.
    fn set_int(&mut self, section: &str, key: &str, value: i32);

    /// Store a floating-point value.
    fn set_float(&mut self, section: &str, key: &str, value: f32);

    /// Store a boolean value.
    fn set_bool(&mut self, section: &str, key: &str, value: bool);

    /// Store a string value.
    fn set_string(&mut self, section: &str, key: &str, value: &str);

    /// Create a new (empty) section. Returns `true` if a new section
    /// was added, `false` if it already existed.
    fn add_section(&mut self, section: &str) -> bool;

    /// Remove an entire section. Returns `true` if the section
    /// existed and was removed, `false` otherwise.
    fn remove_section(&mut self, section: &str) -> bool;
}
