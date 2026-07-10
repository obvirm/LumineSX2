//! SimpleIni - idiomatic Rust 2021 translation of the SimpleIni library.
//!
//! The original SimpleIni library parses and writes INI-style configuration
//! files. This module exposes a much smaller but complete subset of the
//! original API:
//!
//! - `IniFile::load` / `IniFile::save` for I/O.
//! - `IniFile::set_value` / `IniFile::get_value` for mutation and lookup.
//! - The internal storage is a `BTreeMap<String, BTreeMap<String, String>>`
//!   as required by the spec.
//!
//! The parser accepts both Unix (`\n`) and Windows (`\r\n`) line endings
//! and ignores leading/trailing whitespace on section and key names.
//! Comments start with `;` or `#`.

#![allow(dead_code)]
#![allow(non_snake_case)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

/// A single INI configuration, kept as a section -> (key -> value) map.
///
/// Sections are stored in alphabetical order via the `BTreeMap` of
/// strings; keys within a section are also ordered alphabetically.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IniFile {
    pub sections: BTreeMap<String, BTreeMap<String, String>>,
}

impl IniFile {
    /// Construct an empty `IniFile`.
    pub fn new() -> Self {
        IniFile::default()
    }

    /// Load an INI file from disk.
    pub fn load<P: AsRef<Path>>(path: P) -> io::Result<IniFile> {
        let mut s = String::new();
        let mut f = fs::File::open(path)?;
        f.read_to_string(&mut s)?;
        Ok(Self::parse(&s))
    }

    /// Save the `IniFile` to disk. Sections and keys are written in
    /// alphabetical order (the same order the in-memory `BTreeMap`
    /// gives).
    pub fn save<P: AsRef<Path>>(&self, path: P) -> io::Result<()> {
        let mut f = fs::File::create(path)?;
        for (section, kvs) in &self.sections {
            writeln!(f, "[{}]", section)?;
            for (k, v) in kvs {
                writeln!(f, "{}={}", k, v)?;
            }
            writeln!(f)?;
        }
        Ok(())
    }

    /// Parse an INI file from a string.
    pub fn parse(s: &str) -> IniFile {
        let mut ini = IniFile::new();
        let mut current = String::new();
        let mut has_section = false;

        for raw_line in s.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if let Some(stripped) = line.strip_prefix('[') {
                if let Some(name) = stripped.strip_suffix(']') {
                    current = name.trim().to_string();
                    has_section = true;
                    ini.sections.entry(current.clone()).or_default();
                    continue;
                }
            }
            if let Some(eq) = line.find('=') {
                let key = line[..eq].trim().to_string();
                let value = line[eq + 1..].trim().to_string();
                if !has_section {
                    current.clear();
                    has_section = true;
                }
                ini.sections
                    .entry(current.clone())
                    .or_default()
                    .insert(key, value);
            }
        }
        ini
    }

    /// Serialize the `IniFile` to a string.
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for (section, kvs) in &self.sections {
            out.push('[');
            out.push_str(section);
            out.push(']');
            out.push('\n');
            for (k, v) in kvs {
                out.push_str(k);
                out.push('=');
                out.push_str(v);
                out.push('\n');
            }
            out.push('\n');
        }
        out
    }

    /// Set `value` for `key` in `section`. Creates the section/key as
    /// needed. Returns the previous value if any.
    pub fn set_value(
        &mut self,
        section: &str,
        key: &str,
        value: &str,
    ) -> Option<String> {
        self.sections
            .entry(section.to_string())
            .or_default()
            .insert(key.to_string(), value.to_string())
    }

    /// Look up `key` in `section`. Returns `None` if either the section
    /// or the key is missing.
    pub fn get_value(&self, section: &str, key: &str) -> Option<&str> {
        self.sections
            .get(section)?
            .get(key)
            .map(String::as_str)
    }

    /// Returns the names of all sections in alphabetical order.
    pub fn sections(&self) -> impl Iterator<Item = &str> {
        self.sections.keys().map(String::as_str)
    }

    /// Returns the names of all keys in `section`, in alphabetical
    /// order.
    pub fn keys(
        &self,
        section: &str,
    ) -> Option<impl Iterator<Item = &str>> {
        self.sections
            .get(section)
            .map(|kv| kv.keys().map(String::as_str))
    }

    /// Remove a key from a section. Returns the previous value if any.
    pub fn delete_key(&mut self, section: &str, key: &str) -> Option<String> {
        self.sections.get_mut(section)?.remove(key)
    }

    /// Remove a section and all of its keys. Returns the removed
    /// section if any.
    pub fn delete_section(
        &mut self,
        section: &str,
    ) -> Option<BTreeMap<String, String>> {
        self.sections.remove(section)
    }
}
