//! Settings wrapper module.
//!
//! Provides idiomatic Rust wrappers around PCSX2's INI, layered, and
//! emu-folder settings. The module exposes:
//!
//! - [`SettingsInterface`]: a trait abstracting the various back-ends
//!   (in-memory, INI file, layered file, etc.).
//! - [`Settings`]: a concrete handle pairing a back-end with an optional
//!   on-disk folder.
//! - [`get_default_layered_settings_folder`]: returns the canonical
//!   location for the layered settings files.
//! - [`get_toml_path_for_section`]: resolves a section name to the TOML
//!   file that holds its contents in the layered layout.
//!
//! The original C++ split the read, write, and clear halves across
//! `SettingsLoadWrapper`, `SettingsSaveWrapper`, and `SettingsClearWrapper`
//! classes that all derived from an abstract `SettingsWrapper`. In
//! idiomatic Rust those concerns are unified behind a single
//! `SettingsInterface` trait so callers can share one handle across
//! load, save, and clear operations without juggling three wrappers.

use std::path::PathBuf;

/// A trait abstracting the read, write, and delete operations that the
/// various PCSX2 settings back-ends (INI, layered, emu-folder) expose.
///
/// `get_*` methods read a value, falling back to `default` when the
/// key is absent (or, for `get_string_value`, reporting absence through
/// the boolean return). `set_*` methods write a value, and
/// `delete_value` removes a key.
pub trait SettingsInterface {
    fn get_int_value(&self, section: &str, key: &str, default: i32) -> i32;
    fn get_uint_value(&self, section: &str, key: &str, default: u32) -> u32;
    fn get_bool_value(&self, section: &str, key: &str, default: bool) -> bool;
    fn get_float_value(&self, section: &str, key: &str, default: f32) -> f32;

    /// Reads a string value. Returns `true` if a value was present in
    /// the back-end; otherwise `dest` is left untouched and `false`
    /// is returned (matching the C++ `GetStringValue` contract).
    fn get_string_value(&self, section: &str, key: &str, dest: &mut String) -> bool;

    fn set_int_value(&mut self, section: &str, key: &str, value: i32);
    fn set_uint_value(&mut self, section: &str, key: &str, value: u32);
    fn set_bool_value(&mut self, section: &str, key: &str, value: bool);
    fn set_float_value(&mut self, section: &str, key: &str, value: f32);
    fn set_string_value(&mut self, section: &str, key: &str, value: &str);
    fn delete_value(&mut self, section: &str, key: &str);
}

/// A concrete settings handle: a heap-allocated back-end plus an optional
/// folder on disk that back-ends like the layered or emu-folder layouts
/// need to locate their per-section files.
pub struct Settings {
    pub ini: Box<dyn SettingsInterface>,
    pub folder: Option<PathBuf>,
}

/// Returns the default on-disk location for the layered settings files.
///
/// Resolves the platform-appropriate user-config directory: prefers
/// `$XDG_CONFIG_HOME/pcsx2`, then falls back to `$HOME/.config/pcsx2`.
/// Returns `None` if no usable base directory can be determined.
pub fn get_default_layered_settings_folder() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let home = std::env::var_os("HOME")?;
            Some(PathBuf::from(home).join(".config"))
        })?;
    Some(base.join("pcsx2"))
}

/// Resolves the TOML file path that holds `section` in the layered layout.
///
/// Returns `None` for empty section names (a section name is required
/// to form a valid file name) or when the default folder itself cannot
/// be determined.
pub fn get_toml_path_for_section(section: &str) -> Option<PathBuf> {
    if section.is_empty() {
        return None;
    }
    let folder = get_default_layered_settings_folder()?;
    Some(folder.join(format!("{section}.toml")))
}
